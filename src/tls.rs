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

use x509_parser::prelude::*;

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

/// Extract SAN DNS names from a PEM certificate file.
pub fn extract_san_dns_names_from_pem(tls_config: &TlsConfig) -> Vec<String> {
    let cert_path = match tls_config.cert.as_deref() {
        Some(p) => p,
        None => return vec![],
    };
    let cert_pem = match fs::read(cert_path) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let certs: Vec<CertificateDer> = match rustls_pemfile::certs(&mut &*cert_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    if let Some(cert) = certs.first() {
        extract_san_dns_names(cert.as_ref())
    } else {
        vec![]
    }
}

/// Extract SAN (DNS) entries from a DER-encoded certificate.
pub fn extract_san_dns_names(cert_der: &[u8]) -> Vec<String> {
    let (_, cert) = match X509Certificate::from_der(cert_der) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut names = Vec::new();
    for ext in cert.extensions() {
        if let ParsedExtension::SubjectAlternativeName(san) = ext.parsed_extension() {
            for name in &san.general_names {
                if let GeneralName::DNSName(dns) = name {
                    names.push(dns.to_string());
                }
            }
        }
    }
    names
}

/// Check if an SNI hostname matches any SAN pattern.
/// Wildcard matching: "*.localhost" matches "foo.localhost" but not "a.b.localhost".
pub fn matches_san(sni: &str, san_list: &[String]) -> bool {
    let sni_lower = sni.to_lowercase();
    for pattern in san_list {
        let pat_lower = pattern.to_lowercase();
        if pat_lower == sni_lower {
            return true;
        }
        if let Some(suffix) = pat_lower.strip_prefix("*.") {
            // Wildcard: match exactly one level
            if let Some(prefix) = sni_lower.strip_suffix(&format!(".{}", suffix)) {
                if !prefix.contains('.') && !prefix.is_empty() {
                    return true;
                }
            }
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_san_exact() {
        let san = vec!["foo.localhost".to_string()];
        assert!(matches_san("foo.localhost", &san));
        assert!(!matches_san("bar.localhost", &san));
    }

    #[test]
    fn test_matches_san_wildcard_single_level() {
        let san = vec!["*.localhost".to_string()];
        assert!(matches_san("foo.localhost", &san));
        assert!(matches_san("bar.localhost", &san));
        assert!(!matches_san("a.b.localhost", &san));
        assert!(!matches_san("localhost", &san));
    }

    #[test]
    fn test_matches_san_wildcard_multi_level() {
        let san = vec!["*.localhost".to_string(), "*.echub.localhost".to_string()];
        assert!(matches_san("foo.localhost", &san));
        assert!(matches_san("frontend.echub.localhost", &san));
        assert!(!matches_san("deep.frontend.echub.localhost", &san));
    }

    #[test]
    fn test_matches_san_case_insensitive() {
        let san = vec!["*.LOCALHOST".to_string()];
        assert!(matches_san("Foo.localhost", &san));
        assert!(matches_san("foo.Localhost", &san));
    }

    #[test]
    fn test_matches_san_empty() {
        let san: Vec<String> = vec![];
        assert!(!matches_san("foo.localhost", &san));
    }

    // ---- Scenario tests: SAN coverage for real routing keys ----

    /// Cert with only *.localhost — single-level keys work, multi-level don't.
    /// This is the most common initial setup.
    #[test]
    fn test_scenario_default_cert_coverage() {
        let san = vec!["*.localhost".to_string()];

        // These keys produce SNI like "myapp.localhost" → covered
        assert!(matches_san("myapp.localhost", &san));
        assert!(matches_san("dashboard.localhost", &san));

        // Multi-level keys produce SNI like "image.echub.localhost" → NOT covered
        assert!(!matches_san("image.echub.localhost", &san));
        assert!(!matches_san("api.myapp.localhost", &san));
        assert!(!matches_san("deep.api.myapp.localhost", &san));

        // Base domain itself → NOT covered by wildcard
        assert!(!matches_san("localhost", &san));
    }

    /// Cert regenerated with domains file containing *.localhost and *.echub.localhost.
    /// Simulates after user runs `xargs mkcert < /etc/nameroute/domains`.
    #[test]
    fn test_scenario_regenerated_cert_coverage() {
        let san = vec![
            "*.localhost".to_string(),
            "*.echub.localhost".to_string(),
        ];

        // Single-level keys — covered by *.localhost
        assert!(matches_san("myapp.localhost", &san));

        // echub sub-keys — covered by *.echub.localhost
        assert!(matches_san("image.echub.localhost", &san));
        assert!(matches_san("api.echub.localhost", &san));

        // Three-level key under echub — still NOT covered
        assert!(!matches_san("v2.image.echub.localhost", &san));
    }

    /// Verify that the wildcard pattern from domains::wildcard_for_key
    /// actually covers the SNI that would be generated for that routing key.
    #[test]
    fn test_scenario_wildcard_for_key_covers_sni() {
        let base = "localhost";
        let cases = vec![
            // (routing_key, expected_sni)
            ("myapp", "myapp.localhost"),
            ("dashboard", "dashboard.localhost"),
            ("image.echub", "image.echub.localhost"),
            ("api.myapp", "api.myapp.localhost"),
            ("api.frontend.echub", "api.frontend.echub.localhost"),
        ];

        for (key, sni) in &cases {
            let pattern = crate::domains::wildcard_for_key(key, base);
            let san = vec![pattern.clone()];
            assert!(
                matches_san(sni, &san),
                "wildcard_for_key({:?}) = {:?} should cover SNI {:?}",
                key,
                pattern,
                sni
            );
        }
    }

    /// Full scenario: register multiple routes, collect all patterns
    /// from wildcard_for_key, and verify every route's SNI is covered.
    #[test]
    fn test_scenario_all_routes_covered_by_domains_file() {
        let base = "localhost";
        let keys = vec![
            "myapp",
            "dashboard",
            "image.echub",
            "frontend.echub",
            "api.frontend.echub",
        ];

        // Collect unique patterns (simulating domains file)
        let mut patterns: Vec<String> = Vec::new();
        let base_pattern = format!("*.{}", base);
        if !patterns.contains(&base_pattern) {
            patterns.push(base_pattern.clone());
        }
        for key in &keys {
            let pattern = crate::domains::wildcard_for_key(key, base);
            if !patterns.contains(&pattern) {
                patterns.push(pattern);
            }
        }

        // Verify every key's SNI is covered
        for key in &keys {
            let sni = format!("{}.{}", key, base);
            assert!(
                matches_san(&sni, &patterns),
                "SNI {:?} is NOT covered by patterns {:?}",
                sni,
                patterns
            );
        }
    }

    /// Cert with *.localhost should NOT cover multi-level subdomains.
    /// This was the original user-facing problem that motivated this feature.
    #[test]
    fn test_scenario_mismatch_detected_for_uncovered_sni() {
        let san = vec!["*.localhost".to_string()];

        // These multi-level SNIs should NOT match — mismatch page should appear
        let uncovered = vec![
            "image.echub.localhost",
            "api.myapp.localhost",
            "frontend.echub.localhost",
        ];
        for sni in &uncovered {
            assert!(
                !matches_san(sni, &san),
                "SNI {:?} should NOT be covered by {:?}",
                sni,
                san
            );
        }
    }

    // ---- Edge case tests: ClientHello parsing ----

    /// Invalid TLS record type (not handshake).
    #[test]
    fn test_parse_sni_not_client_hello() {
        // handshake type = 2 (ServerHello, not ClientHello)
        let data = vec![2, 0, 0, 0];
        let result = parse_sni_from_client_hello(&data);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Not a ClientHello"),
        );
    }

    /// Truncated handshake data.
    #[test]
    fn test_parse_sni_truncated() {
        let data = vec![1, 0]; // ClientHello type but too short
        let result = parse_sni_from_client_hello(&data);
        assert!(result.is_err());
    }

    /// Empty data.
    #[test]
    fn test_parse_sni_empty() {
        let data: Vec<u8> = vec![];
        let result = parse_sni_from_client_hello(&data);
        assert!(result.is_err());
    }

    // ---- Edge case tests: SAN matching boundary conditions ----

    /// Wildcard should not match the base domain itself.
    #[test]
    fn test_matches_san_wildcard_does_not_match_base() {
        let san = vec!["*.localhost".to_string()];
        assert!(!matches_san("localhost", &san));
    }

    /// Wildcard should not match empty prefix.
    #[test]
    fn test_matches_san_wildcard_empty_prefix() {
        let san = vec!["*.localhost".to_string()];
        assert!(!matches_san(".localhost", &san));
    }

    /// Exact match (non-wildcard) SAN entry.
    #[test]
    fn test_matches_san_exact_non_wildcard() {
        let san = vec!["myapp.localhost".to_string()];
        assert!(matches_san("myapp.localhost", &san));
        assert!(!matches_san("other.localhost", &san));
    }

    /// Mixed exact and wildcard entries.
    #[test]
    fn test_matches_san_mixed_exact_and_wildcard() {
        let san = vec![
            "specific.example.com".to_string(),
            "*.localhost".to_string(),
        ];
        assert!(matches_san("specific.example.com", &san));
        assert!(matches_san("foo.localhost", &san));
        assert!(!matches_san("other.example.com", &san));
    }

    // ---- End-to-end scenario: extract_subdomain → SNI → SAN check ----

    /// Verify the full chain: routing key → SNI hostname → SAN match/mismatch.
    /// This tests the interaction between http::extract_subdomain and tls::matches_san.
    #[test]
    fn test_scenario_subdomain_to_sni_to_san_chain() {
        use crate::protocol::http::extract_subdomain;

        let base = "localhost";
        let san_default = vec!["*.localhost".to_string()];
        let san_extended = vec![
            "*.localhost".to_string(),
            "*.echub.localhost".to_string(),
        ];

        // Case 1: Simple key — covered by default cert
        let sni = "myapp.localhost";
        let key = extract_subdomain(sni, base);
        assert_eq!(key, Some("myapp".to_string()));
        assert!(matches_san(sni, &san_default));

        // Case 2: Multi-level key — NOT covered by default cert
        let sni = "image.echub.localhost";
        let key = extract_subdomain(sni, base);
        assert_eq!(key, Some("image.echub".to_string()));
        assert!(!matches_san(sni, &san_default));

        // Case 3: Same multi-level key — covered after cert regeneration
        assert!(matches_san(sni, &san_extended));

        // Case 4: Even deeper key — not covered even by extended cert
        let sni = "v2.image.echub.localhost";
        let key = extract_subdomain(sni, base);
        assert_eq!(key, Some("v2.image.echub".to_string()));
        assert!(!matches_san(sni, &san_extended));
    }
}
