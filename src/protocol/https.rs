use std::net::SocketAddr;

use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use crate::config::{Config, TlsConfig};
use crate::error::Result;
use crate::protocol::http::{self, extract_subdomain, send_html_response};
use crate::protocol::{ProtocolHandler, ProtocolKind, TlsMode};
use crate::proxy;
use crate::router::SharedRoutingTable;
use crate::tls;

pub struct HttpsHandler {
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
    tls_acceptor: Option<TlsAcceptor>,
    san_names: Vec<String>,
    tls_config: TlsConfig,
}

impl HttpsHandler {
    pub fn new(
        routing_table: SharedRoutingTable,
        config_rx: watch::Receiver<Config>,
        tls_acceptor: Option<TlsAcceptor>,
        san_names: Vec<String>,
        tls_config: TlsConfig,
    ) -> Self {
        Self {
            routing_table,
            config_rx,
            tls_acceptor,
            san_names,
            tls_config,
        }
    }
}

impl ProtocolHandler for HttpsHandler {
    async fn handle_connection(&self, mut client: TcpStream, peer: SocketAddr) -> Result<()> {
        debug!(peer = %peer, "New HTTPS connection");

        // 1. Read ClientHello and extract SNI
        let (server_name, buffer) = tls::read_sni(&mut client).await?;
        debug!(peer = %peer, sni = %server_name, "SNI extracted");

        // 2. Extract subdomain from SNI
        let config = self.config_rx.borrow().clone();
        let base_domain = &config.http.base_domain;
        let key = extract_subdomain(&server_name, base_domain);

        let key = match key {
            Some(k) if !k.is_empty() => k,
            _ => {
                debug!(peer = %peer, sni = %server_name, "No subdomain in SNI");
                return Ok(());
            }
        };

        // 3. Lookup in routing table
        let table = self.routing_table.read().await;
        let backend = table.lookup(ProtocolKind::Https, &key).cloned();
        drop(table);

        let backend = match backend {
            Some(b) => b,
            None => {
                info!(peer = %peer, key = %key, "HTTPS backend not found");
                return Ok(());
            }
        };

        info!(
            peer = %peer,
            key = %key,
            backend = %backend.container_name,
            tls_mode = %backend.tls_mode,
            "Routing HTTPS connection"
        );

        match backend.tls_mode {
            TlsMode::Passthrough => {
                // Connect to backend and forward the ClientHello + bidirectional copy
                let mut backend_stream =
                    proxy::connect_backend(&backend, &config.backend).await?;
                backend_stream.write_all(&buffer).await?;

                match copy_bidirectional(&mut client, &mut backend_stream).await {
                    Ok((c2b, b2c)) => {
                        debug!(
                            peer = %peer,
                            client_to_backend = c2b,
                            backend_to_client = b2c,
                            "HTTPS passthrough connection closed"
                        );
                    }
                    Err(e) => {
                        debug!(peer = %peer, error = %e, "HTTPS passthrough relay ended");
                    }
                }
            }
            TlsMode::Terminate => {
                let tls_acceptor = match &self.tls_acceptor {
                    Some(a) => a.clone(),
                    None => {
                        warn!(peer = %peer, "TLS terminate requested but no TLS acceptor configured. Add [tls] cert/key to config.");
                        return Ok(());
                    }
                };

                // Wrap the stream with prefix buffer so TLS acceptor can re-read the ClientHello
                let prefixed = tls::PrefixedStream::new(buffer, client);
                let mut tls_stream = tls_acceptor.accept(prefixed).await.map_err(|e| {
                    debug!(peer = %peer, error = %e, "TLS handshake failed");
                    e
                })?;

                // Check SAN coverage after TLS handshake
                if !self.san_names.is_empty()
                    && !tls::matches_san(&server_name, &self.san_names)
                {
                    warn!(
                        peer = %peer,
                        sni = %server_name,
                        "Certificate does not cover SNI (SAN mismatch)"
                    );

                    // Only show detailed diagnostic info (file paths, commands)
                    // to connections from loopback addresses.
                    let is_local = peer.ip().is_loopback();

                    let body = if is_local {
                        let cert_path = self
                            .tls_config
                            .cert
                            .as_deref()
                            .unwrap_or("/etc/nameroute/cert.pem");
                        let key_path = self
                            .tls_config
                            .key
                            .as_deref()
                            .unwrap_or("/etc/nameroute/key.pem");

                        let san_list_html: String = self
                            .san_names
                            .iter()
                            .map(|s| format!("  <li><code>{}</code></li>\n", s))
                            .collect();

                        format!(
                            r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Certificate Error</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 700px; margin: 40px auto; padding: 0 20px; color: #333; }}
  h1 {{ color: #222; font-size: 1.6em; border-bottom: 2px solid #ccc; padding-bottom: 8px; }}
  h2.warn {{ color: #c00; }}
  pre {{ background: #f4f4f4; padding: 16px; border-radius: 6px; overflow-x: auto; }}
  code {{ background: #f4f4f4; padding: 2px 6px; border-radius: 3px; }}
  ul {{ line-height: 1.8; }}
</style>
</head>
<body>
<h1>name-route</h1>
<h2 class="warn">&#x26A0; Certificate does not cover &ldquo;{sni}&rdquo;</h2>
<p>The TLS certificate loaded by name-route does not include a SAN entry
that matches <code>{sni}</code>.</p>
<h2>Current certificate covers:</h2>
<ul>
{san_list_html}</ul>
<h2>To fix, regenerate the certificate:</h2>
<pre>sudo xargs mkcert \
  -key-file {key_path} \
  -cert-file {cert_path} \
  &lt; /etc/nameroute/domains

sudo systemctl restart nameroute</pre>
</body>
</html>"#,
                            sni = server_name,
                            san_list_html = san_list_html,
                            key_path = key_path,
                            cert_path = cert_path,
                        )
                    } else {
                        format!(
                            r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Certificate Error</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 700px; margin: 40px auto; padding: 0 20px; color: #333; }}
  h1 {{ color: #222; font-size: 1.6em; border-bottom: 2px solid #ccc; padding-bottom: 8px; }}
  h2.warn {{ color: #c00; }}
</style>
</head>
<body>
<h1>name-route</h1>
<h2 class="warn">&#x26A0; Certificate does not cover &ldquo;{sni}&rdquo;</h2>
<p>The TLS certificate does not include a SAN entry
that matches <code>{sni}</code>. Please contact the server administrator.</p>
</body>
</html>"#,
                            sni = server_name,
                        )
                    };

                    let _ = send_html_response(
                        &mut tls_stream,
                        421,
                        "Misdirected Request",
                        &body,
                    )
                    .await;
                    return Ok(());
                }

                // Delegate to HTTP handler with ProtocolKind::Https for routing lookup.
                // Pass the SNI-derived key to verify Host header matches,
                // preventing host header attacks.
                http::handle_http_stream(
                    tls_stream,
                    peer,
                    &self.routing_table,
                    &self.config_rx,
                    ProtocolKind::Https,
                    Some(&key),
                )
                .await?;
            }
        }

        Ok(())
    }
}
