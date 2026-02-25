use std::net::SocketAddr;

use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::Result;
use crate::protocol::http::{self, extract_subdomain};
use crate::protocol::{ProtocolHandler, ProtocolKind, TlsMode};
use crate::proxy;
use crate::router::SharedRoutingTable;
use crate::tls;

pub struct HttpsHandler {
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
    tls_acceptor: Option<TlsAcceptor>,
}

impl HttpsHandler {
    pub fn new(
        routing_table: SharedRoutingTable,
        config_rx: watch::Receiver<Config>,
        tls_acceptor: Option<TlsAcceptor>,
    ) -> Self {
        Self {
            routing_table,
            config_rx,
            tls_acceptor,
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
                let tls_stream = tls_acceptor.accept(prefixed).await.map_err(|e| {
                    debug!(peer = %peer, error = %e, "TLS handshake failed");
                    e
                })?;

                // Delegate to HTTP handler with ProtocolKind::Https for routing lookup
                http::handle_http_stream(
                    tls_stream,
                    peer,
                    &self.routing_table,
                    &self.config_rx,
                    ProtocolKind::Https,
                )
                .await?;
            }
        }

        Ok(())
    }
}
