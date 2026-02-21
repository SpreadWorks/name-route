use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::ListenerConfig;
use crate::error::Result;
use crate::protocol::ProtocolHandler;

/// Run a TCP listener loop for a given protocol handler.
pub async fn run_listener<H: ProtocolHandler>(
    config: &ListenerConfig,
    handler: Arc<H>,
    cancel: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(&config.bind).await?;
    info!(
        protocol = %config.protocol,
        bind = %config.bind,
        "Listener started"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!(protocol = %config.protocol, "Listener shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, peer)) => {
                        let handler = Arc::clone(&handler);
                        tokio::spawn(async move {
                            if let Err(e) = handler.handle_connection(stream, peer).await {
                                error!(peer = %peer, error = %e, "Connection handler error");
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "Accept error");
                    }
                }
            }
        }
    }

    Ok(())
}
