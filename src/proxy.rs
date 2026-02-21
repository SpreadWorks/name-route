use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::config::BackendConfig;
use crate::error::{Error, Result};
use crate::router::Backend;

/// Connect to a backend, trying each address in order with retries.
pub async fn connect_backend(backend: &Backend, config: &BackendConfig) -> Result<TcpStream> {
    let timeout = Duration::from_secs(config.connect_timeout);

    for attempt in 0..config.connect_retries {
        for addr in &backend.addrs {
            let sock_addr = SocketAddr::new(*addr, backend.port);
            debug!(
                attempt = attempt + 1,
                addr = %sock_addr,
                container = %backend.container_name,
                "Connecting to backend"
            );

            match tokio::time::timeout(timeout, TcpStream::connect(sock_addr)).await {
                Ok(Ok(stream)) => {
                    info!(
                        addr = %sock_addr,
                        container = %backend.container_name,
                        "Connected to backend"
                    );
                    return Ok(stream);
                }
                Ok(Err(e)) => {
                    warn!(
                        addr = %sock_addr,
                        attempt = attempt + 1,
                        error = %e,
                        "Backend connect failed"
                    );
                }
                Err(_) => {
                    warn!(
                        addr = %sock_addr,
                        attempt = attempt + 1,
                        "Backend connect timed out"
                    );
                }
            }
        }
    }

    Err(Error::Connection(format!(
        "Failed to connect to backend {} after {} retries",
        backend.container_name, config.connect_retries
    )))
}

/// Relay data bidirectionally between client and backend.
/// If there is buffered data from protocol parsing, send it to backend first.
pub async fn relay(
    mut client: TcpStream,
    mut backend: TcpStream,
    buffered_data: Option<&[u8]>,
    peer: SocketAddr,
) -> Result<()> {
    if let Some(data) = buffered_data {
        if !data.is_empty() {
            debug!(peer = %peer, bytes = data.len(), "Sending buffered data to backend");
            backend.write_all(data).await?;
        }
    }

    match copy_bidirectional(&mut client, &mut backend).await {
        Ok((client_to_backend, backend_to_client)) => {
            debug!(
                peer = %peer,
                client_to_backend,
                backend_to_client,
                "Connection closed"
            );
            Ok(())
        }
        Err(e) => {
            debug!(peer = %peer, error = %e, "Relay ended");
            Ok(())
        }
    }
}

/// Helper: connect to backend and relay, sending the initial message first.
pub async fn connect_and_relay(
    client: TcpStream,
    backend: &Backend,
    config: &BackendConfig,
    initial_data: &[u8],
    peer: SocketAddr,
) -> Result<()> {
    let backend_stream = connect_backend(backend, config).await?;
    relay(client, backend_stream, Some(initial_data), peer).await
}
