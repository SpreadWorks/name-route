use std::fs;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

use crate::config::TlsConfig;
use crate::error::{Error, Result};

pub fn create_tls_acceptor(tls_config: &TlsConfig) -> Result<TlsAcceptor> {
    let cert_path = tls_config
        .cert
        .as_deref()
        .ok_or_else(|| Error::Config("TLS cert path is not configured".to_string()))?;
    let key_path = tls_config
        .key
        .as_deref()
        .ok_or_else(|| Error::Config("TLS key path is not configured".to_string()))?;

    let cert_pem = fs::read(cert_path).map_err(|e| {
        Error::Config(format!("Failed to read TLS cert {:?}: {}", cert_path, e))
    })?;
    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut &*cert_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Config(format!("Failed to parse TLS cert PEM: {}", e)))?;
    if certs.is_empty() {
        return Err(Error::Config(format!(
            "No certificates found in {:?}",
            cert_path
        )));
    }

    let key_pem = fs::read(key_path).map_err(|e| {
        Error::Config(format!("Failed to read TLS key {:?}: {}", key_path, e))
    })?;
    let key: PrivateKeyDer = rustls_pemfile::private_key(&mut &*key_pem)
        .map_err(|e| Error::Config(format!("Failed to parse TLS key PEM: {}", e)))?
        .ok_or_else(|| {
            Error::Config(format!("No private key found in {:?}", key_path))
        })?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| Error::Config(format!("Failed to build TLS config: {}", e)))?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Read the TLS ClientHello from a TCP stream and extract the SNI server name.
/// Returns the server name and the raw bytes read (for replay to backend or TLS acceptor).
pub async fn read_sni(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    use tokio::io::AsyncReadExt;

    // Read TLS record header (5 bytes): content_type(1) + version(2) + length(2)
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).await.map_err(|e| {
        Error::Connection(format!("Failed to read TLS record header: {}", e))
    })?;

    if header[0] != 22 {
        return Err(Error::Connection(format!(
            "Not a TLS handshake record (type={})",
            header[0]
        )));
    }

    let record_len = u16::from_be_bytes([header[3], header[4]]) as usize;
    if record_len == 0 || record_len > 16384 {
        return Err(Error::Connection(format!(
            "Invalid TLS record length: {}",
            record_len
        )));
    }

    // Read the full handshake message
    let mut payload = vec![0u8; record_len];
    stream.read_exact(&mut payload).await.map_err(|e| {
        Error::Connection(format!("Failed to read TLS handshake payload: {}", e))
    })?;

    // Combine header + payload into the buffer we'll return
    let mut buffer = Vec::with_capacity(5 + record_len);
    buffer.extend_from_slice(&header);
    buffer.extend_from_slice(&payload);

    // Parse ClientHello to extract SNI
    let server_name = parse_sni_from_client_hello(&payload)?;

    Ok((server_name, buffer))
}

/// Parse SNI server name from a TLS handshake payload (after record header).
fn parse_sni_from_client_hello(data: &[u8]) -> Result<String> {
    // Handshake header: type(1) + length(3)
    if data.len() < 4 {
        return Err(Error::Connection("Handshake message too short".to_string()));
    }
    if data[0] != 1 {
        return Err(Error::Connection(format!(
            "Not a ClientHello (handshake type={})",
            data[0]
        )));
    }

    let mut pos = 4; // skip handshake type + length

    // ClientHello: version(2) + random(32) = 34 bytes
    if data.len() < pos + 34 {
        return Err(Error::Connection("ClientHello too short".to_string()));
    }
    pos += 34;

    // Session ID
    if data.len() < pos + 1 {
        return Err(Error::Connection("ClientHello truncated at session_id_len".to_string()));
    }
    let session_id_len = data[pos] as usize;
    pos += 1 + session_id_len;

    // Cipher suites
    if data.len() < pos + 2 {
        return Err(Error::Connection("ClientHello truncated at cipher_suites_len".to_string()));
    }
    let cipher_suites_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2 + cipher_suites_len;

    // Compression methods
    if data.len() < pos + 1 {
        return Err(Error::Connection("ClientHello truncated at compression_len".to_string()));
    }
    let compression_len = data[pos] as usize;
    pos += 1 + compression_len;

    // Extensions
    if data.len() < pos + 2 {
        return Err(Error::Connection("No extensions in ClientHello".to_string()));
    }
    let extensions_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;

    let extensions_end = pos + extensions_len;
    if data.len() < extensions_end {
        return Err(Error::Connection("Extensions truncated".to_string()));
    }

    while pos + 4 <= extensions_end {
        let ext_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let ext_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if ext_type == 0 {
            // SNI extension
            // SNI list: list_len(2) [ type(1) name_len(2) name(...) ]*
            if ext_len < 5 {
                return Err(Error::Connection("SNI extension too short".to_string()));
            }
            let _list_len = u16::from_be_bytes([data[pos], data[pos + 1]]);
            let name_type = data[pos + 2];
            if name_type != 0 {
                // type 0 = hostname
                pos += ext_len;
                continue;
            }
            let name_len = u16::from_be_bytes([data[pos + 3], data[pos + 4]]) as usize;
            if data.len() < pos + 5 + name_len {
                return Err(Error::Connection("SNI name truncated".to_string()));
            }
            let name = String::from_utf8_lossy(&data[pos + 5..pos + 5 + name_len]).to_string();
            return Ok(name);
        }

        pos += ext_len;
    }

    Err(Error::Connection("No SNI extension found in ClientHello".to_string()))
}

/// A stream that replays a prefix buffer before reading from the inner stream.
/// Used to feed already-read ClientHello bytes back into the TLS acceptor.
pub struct PrefixedStream {
    prefix: Vec<u8>,
    pos: usize,
    inner: TcpStream,
}

impl PrefixedStream {
    pub fn new(prefix: Vec<u8>, inner: TcpStream) -> Self {
        Self {
            prefix,
            pos: 0,
            inner,
        }
    }
}

impl AsyncRead for PrefixedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        if this.pos < this.prefix.len() {
            let remaining = &this.prefix[this.pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            this.pos += to_copy;
            return Poll::Ready(Ok(()));
        }

        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
