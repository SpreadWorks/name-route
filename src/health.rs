use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::config::Config;
use crate::router::{HealthStatus, SharedHealthMap, SharedRoutingTable};

pub async fn polling_loop(
    routing_table: SharedRoutingTable,
    health_map: SharedHealthMap,
    config_rx: watch::Receiver<Config>,
    cancel: CancellationToken,
) {
    let config_rx = config_rx;
    let mut interval_secs = config_rx.borrow().health_check.interval;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Health check polling loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let config = config_rx.borrow().clone();
                if !config.health_check.enabled {
                    continue;
                }

                // Update interval if config changed
                if config.health_check.interval != interval_secs {
                    interval_secs = config.health_check.interval;
                    interval = tokio::time::interval(Duration::from_secs(interval_secs));
                }

                let timeout = Duration::from_secs(config.health_check.timeout);

                // Snapshot current routes
                let snapshot: Vec<_> = {
                    let table = routing_table.read().await;
                    table
                        .entries()
                        .map(|((protocol, key), backend)| {
                            let addr = backend
                                .addrs
                                .first()
                                .map(|a| SocketAddr::new(*a, backend.port));
                            ((*protocol, key.clone()), addr)
                        })
                        .collect()
                };

                // Check each backend concurrently
                let mut handles = Vec::with_capacity(snapshot.len());
                for (route_key, addr) in snapshot {
                    let timeout = timeout;
                    handles.push(tokio::spawn(async move {
                        let status = match addr {
                            Some(addr) => {
                                match tokio::time::timeout(timeout, TcpStream::connect(addr)).await
                                {
                                    Ok(Ok(_)) => HealthStatus::Healthy,
                                    _ => HealthStatus::Unhealthy,
                                }
                            }
                            None => HealthStatus::Unhealthy,
                        };
                        (route_key, status)
                    }));
                }

                // Collect results and rebuild health map
                let mut new_map = std::collections::HashMap::new();
                for handle in handles {
                    if let Ok((route_key, status)) = handle.await {
                        debug!(protocol = %route_key.0, key = %route_key.1, status = ?status, "Health check result");
                        new_map.insert(route_key, status);
                    }
                }

                // Replace the entire map (stale entries for removed routes are cleaned up)
                *health_map.write().await = new_map;
            }
        }
    }
}
