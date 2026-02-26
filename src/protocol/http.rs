use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::Result;
use crate::protocol::ProtocolHandler;
use crate::protocol::ProtocolKind;
use crate::proxy;
use crate::router::SharedRoutingTable;

const MAX_HEADER_SIZE: usize = 8192;

pub struct HttpHandler {
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
}

impl HttpHandler {
    pub fn new(routing_table: SharedRoutingTable, config_rx: watch::Receiver<Config>) -> Self {
        Self {
            routing_table,
            config_rx,
        }
    }
}

impl ProtocolHandler for HttpHandler {
    async fn handle_connection(&self, client: TcpStream, peer: SocketAddr) -> Result<()> {
        handle_http_stream(client, peer, &self.routing_table, &self.config_rx, ProtocolKind::Http, None)
            .await
    }
}

/// HTTP stream handler that works with any stream type (TcpStream, TlsStream, etc.)
/// `protocol_kind` determines which protocol to use for routing table lookup
/// (Http for plain HTTP, Https for TLS-terminated HTTPS).
/// `expected_key` is set for HTTPS terminate mode to verify the Host header matches
/// the SNI-derived key, preventing host header attacks.
pub(crate) async fn handle_http_stream<S>(
    client: S,
    peer: SocketAddr,
    routing_table: &SharedRoutingTable,
    config_rx: &watch::Receiver<Config>,
    protocol_kind: ProtocolKind,
    expected_key: Option<&str>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    debug!(peer = %peer, "New HTTP connection");

    let config = config_rx.borrow().clone();
    let base_domain = &config.http.base_domain;
    let handshake_timeout = Duration::from_secs(config.backend.idle_timeout);

    // Read request line + headers into buffer until \r\n\r\n
    let mut buf = Vec::with_capacity(4096);
    let mut buf_reader = BufReader::new(client);

    let header_read = async {
        loop {
            let available = buf_reader.fill_buf().await?;
            if available.is_empty() {
                return Ok::<Option<usize>, crate::error::Error>(None);
            }

            // Search for \r\n\r\n in the combined existing buf + new data
            let prev_len = buf.len();
            buf.extend_from_slice(available);
            let consumed = available.len();
            buf_reader.consume(consumed);

            if buf.len() > MAX_HEADER_SIZE {
                send_response(buf_reader.get_mut(), 431, "Request Header Fields Too Large", "Headers too large").await?;
                return Ok(None);
            }

            // Search for \r\n\r\n starting from where it could first appear
            let search_start = prev_len.saturating_sub(3);
            if let Some(pos) = buf[search_start..].windows(4).position(|w| w == b"\r\n\r\n") {
                return Ok(Some(search_start + pos + 4));
            }
        }
    };

    let header_end = match tokio::time::timeout(handshake_timeout, header_read).await {
        Ok(Ok(Some(end))) => end,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            warn!(peer = %peer, "HTTP header read timed out");
            return Ok(());
        }
    };

    // Parse Host header from the header portion of the buffer
    let header_str = String::from_utf8_lossy(&buf[..header_end]);
    let host_value = header_str
        .lines()
        .find(|line| {
            let lower = line.to_lowercase();
            lower.starts_with("host:")
        })
        .and_then(|line| line.split_once(':').map(|(_, v)| v.trim().to_string()));

    let host = match &host_value {
        Some(h) => h.as_str(),
        None => {
            send_response(buf_reader.get_mut(), 400, "Bad Request", "Missing Host header").await?;
            return Ok(());
        }
    };

    // Extract subdomain from Host header
    let host_no_port = host.split(':').next().unwrap_or(host);
    let key = extract_subdomain(host_no_port, base_domain);

    let key = match key {
        Some(k) if !k.is_empty() => k,
        _ => {
            send_response(buf_reader.get_mut(), 404, "Not Found", "No subdomain specified").await?;
            return Ok(());
        }
    };

    // In HTTPS terminate mode, verify the Host header matches the SNI-derived key
    // to prevent host header attacks that could route to unintended backends.
    if let Some(expected) = expected_key {
        if key != expected {
            info!(peer = %peer, host_key = %key, sni_key = %expected, "Host header does not match SNI");
            send_response(
                buf_reader.get_mut(),
                421,
                "Misdirected Request",
                "Host header does not match SNI",
            )
            .await?;
            return Ok(());
        }
    }

    debug!(peer = %peer, key = %key, host = %host, "HTTP routing lookup");

    // Lookup in routing table
    let table = routing_table.read().await;
    let backend = table.lookup(protocol_kind, &key).cloned();
    drop(table);

    let backend = match backend {
        Some(b) => b,
        None => {
            info!(peer = %peer, key = %key, "HTTP backend not found");
            send_response(
                buf_reader.get_mut(),
                502,
                "Bad Gateway",
                &format!("No backend for '{}'", key),
            )
            .await?;
            return Ok(());
        }
    };

    info!(
        peer = %peer,
        key = %key,
        backend = %backend.container_name,
        "Routing HTTP connection"
    );

    // Connect to backend and relay
    // Send the already-read data (headers + any extra body bytes), then bidirectional copy
    let mut backend_stream = proxy::connect_backend(&backend, &config.backend).await?;
    backend_stream.write_all(&buf).await?;

    let mut client = buf_reader.into_inner();
    match tokio::time::timeout(
        proxy::MAX_RELAY_DURATION,
        copy_bidirectional(&mut client, &mut backend_stream),
    )
    .await
    {
        Ok(Ok((c2b, b2c))) => {
            debug!(
                peer = %peer,
                client_to_backend = c2b,
                backend_to_client = b2c,
                "HTTP connection closed"
            );
        }
        Ok(Err(e)) => {
            debug!(peer = %peer, error = %e, "HTTP relay ended");
        }
        Err(_) => {
            debug!(peer = %peer, "HTTP relay timed out");
        }
    }

    Ok(())
}

/// Extract subdomain from host given a base domain.
/// "dev1.localhost" with base "localhost" → Some("dev1")
/// "sub.dev1.localhost" with base "localhost" → Some("sub.dev1")
/// "localhost" with base "localhost" → None
pub(crate) fn extract_subdomain(host: &str, base_domain: &str) -> Option<String> {
    let host_lower = host.to_lowercase();
    let base_lower = base_domain.to_lowercase();

    if host_lower == base_lower {
        return None;
    }

    let suffix = format!(".{}", base_lower);
    if host_lower.ends_with(&suffix) {
        let sub = &host_lower[..host_lower.len() - suffix.len()];
        if sub.is_empty() {
            None
        } else {
            Some(sub.to_string())
        }
    } else {
        None
    }
}

pub(crate) async fn send_html_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn send_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subdomain() {
        assert_eq!(
            extract_subdomain("dev1.localhost", "localhost"),
            Some("dev1".to_string())
        );
        assert_eq!(
            extract_subdomain("api1.localhost", "localhost"),
            Some("api1".to_string())
        );
        assert_eq!(
            extract_subdomain("sub.dev1.localhost", "localhost"),
            Some("sub.dev1".to_string())
        );
        assert_eq!(extract_subdomain("localhost", "localhost"), None);
        assert_eq!(extract_subdomain("example.com", "localhost"), None);
        assert_eq!(
            extract_subdomain("DEV1.LOCALHOST", "localhost"),
            Some("dev1".to_string())
        );
    }

    #[test]
    fn test_extract_subdomain_custom_domain() {
        assert_eq!(
            extract_subdomain("app.mysite.local", "mysite.local"),
            Some("app".to_string())
        );
        assert_eq!(extract_subdomain("mysite.local", "mysite.local"), None);
    }

    // ---- Scenario tests: real-world HTTP routing ----

    /// Host header with port (e.g. "myapp.localhost:8080") should extract subdomain correctly.
    #[test]
    fn test_extract_subdomain_host_with_port_stripped() {
        // In handle_http_stream, host_no_port = host.split(':').next()
        // So the port is stripped before calling extract_subdomain.
        let host = "myapp.localhost:8080";
        let host_no_port = host.split(':').next().unwrap_or(host);
        assert_eq!(
            extract_subdomain(host_no_port, "localhost"),
            Some("myapp".to_string())
        );
    }

    /// Multi-level subdomain routing: "api.image.echub.localhost" → key "api.image.echub"
    #[test]
    fn test_extract_subdomain_deep_multilevel() {
        assert_eq!(
            extract_subdomain("api.image.echub.localhost", "localhost"),
            Some("api.image.echub".to_string())
        );
        assert_eq!(
            extract_subdomain("v2.api.image.echub.localhost", "localhost"),
            Some("v2.api.image.echub".to_string())
        );
    }

    /// Empty subdomain edge case: ".localhost" should not produce a key.
    #[test]
    fn test_extract_subdomain_empty_prefix() {
        // ".localhost" does not end with ".localhost" suffix (it IS ".localhost"),
        // and it doesn't equal "localhost" either, so it returns None.
        assert_eq!(extract_subdomain(".localhost", "localhost"), None);
    }

    /// Unrelated domain should return None.
    #[test]
    fn test_extract_subdomain_unrelated_domain() {
        assert_eq!(extract_subdomain("evil.example.com", "localhost"), None);
        assert_eq!(extract_subdomain("notlocalhost", "localhost"), None);
        assert_eq!(extract_subdomain("foolocalhost", "localhost"), None);
    }

    /// Suffix attack: "evil.notlocalhost" should not match base "localhost".
    #[test]
    fn test_extract_subdomain_suffix_attack() {
        assert_eq!(extract_subdomain("evil.notlocalhost", "localhost"), None);
    }
}
