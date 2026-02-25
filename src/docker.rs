use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use bollard::container::ListContainersOptions;
use bollard::Docker;
use serde::Deserialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::domains;
use crate::protocol::{ProtocolKind, TlsMode};
use crate::router::{Backend, RoutingTable, SharedRoutingTable};

const LABEL_KEY: &str = "name-route";

#[derive(Debug, Deserialize)]
struct RouteLabel {
    protocol: ProtocolKind,
    key: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    tls_mode: Option<TlsMode>,
}

/// Connect to Docker with retries.
pub async fn connect_docker(config: &Config) -> crate::error::Result<Docker> {
    let socket = &config.docker.socket;
    let retries = config.docker.startup_retries;
    let interval = Duration::from_secs(config.docker.startup_retry_interval);

    for attempt in 1..=retries {
        match Docker::connect_with_socket(socket, 120, bollard::API_DEFAULT_VERSION) {
            Ok(docker) => {
                // Verify connection by pinging
                match docker.ping().await {
                    Ok(_) => {
                        info!(socket = %socket, "Connected to Docker");
                        return Ok(docker);
                    }
                    Err(e) => {
                        warn!(
                            attempt,
                            retries,
                            error = %e,
                            "Docker ping failed, retrying"
                        );
                    }
                }
            }
            Err(e) => {
                warn!(
                    attempt,
                    retries,
                    error = %e,
                    "Failed to connect to Docker, retrying"
                );
            }
        }

        if attempt < retries {
            tokio::time::sleep(interval).await;
        }
    }

    Err(crate::error::Error::Connection(format!(
        "Failed to connect to Docker at {} after {} retries",
        socket, retries
    )))
}

/// Perform a single polling cycle: list containers and build routing table.
pub async fn poll_once(docker: &Docker) -> crate::error::Result<RoutingTable> {
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![LABEL_KEY.to_string()]);
    filters.insert("status".to_string(), vec!["running".to_string()]);

    let options = ListContainersOptions {
        all: false,
        filters,
        ..Default::default()
    };

    let containers = docker.list_containers(Some(options)).await?;
    let mut table = RoutingTable::new();

    for container in containers {
        let container_id = container.id.as_deref().unwrap_or("unknown").to_string();
        let container_name = container
            .names
            .as_ref()
            .and_then(|names| names.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| container_id.chars().take(12).collect());

        let labels = match &container.labels {
            Some(l) => l,
            None => continue,
        };

        let label_value = match labels.get(LABEL_KEY) {
            Some(v) => v,
            None => continue,
        };

        let routes: Vec<RouteLabel> = match serde_json::from_str(label_value) {
            Ok(r) => r,
            Err(e) => {
                error!(
                    container = %container_name,
                    label = %label_value,
                    error = %e,
                    "Failed to parse name-route label"
                );
                continue;
            }
        };

        // Extract all IP addresses from container networks
        let addrs = extract_container_ips(&container);
        if addrs.is_empty() {
            warn!(
                container = %container_name,
                "Container has no IP addresses, skipping"
            );
            continue;
        }

        for route in routes {
            let port = route.port.unwrap_or_else(|| route.protocol.default_port());

            let backend = Backend {
                source: "docker".to_string(),
                container_name: container_name.clone(),
                addrs: addrs.clone(),
                port,
                tls_mode: route.tls_mode.unwrap_or(TlsMode::Passthrough),
            };

            let collision = table.insert(route.protocol, route.key.clone(), backend);
            if collision {
                warn!(
                    protocol = %route.protocol,
                    key = %route.key,
                    container = %container_name,
                    "Routing key collision, overwriting previous entry"
                );
            }

            debug!(
                protocol = %route.protocol,
                key = %route.key,
                container = %container_name,
                port,
                "Registered route"
            );
        }
    }

    Ok(table)
}

fn extract_container_ips(
    container: &bollard::models::ContainerSummary,
) -> Vec<IpAddr> {
    let mut addrs = Vec::new();

    if let Some(network_settings) = &container.network_settings {
        if let Some(networks) = &network_settings.networks {
            for (_net_name, net) in networks {
                if let Some(ip_str) = &net.ip_address {
                    if !ip_str.is_empty() {
                        if let Ok(ip) = ip_str.parse::<IpAddr>() {
                            addrs.push(ip);
                        }
                    }
                }
            }
        }
    }

    addrs
}

/// Run the Docker polling loop in the background.
pub async fn polling_loop(
    docker: Docker,
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
    cancel: CancellationToken,
) {
    let mut config_rx = config_rx;
    let mut interval_secs = config_rx.borrow().docker.poll_interval;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Docker polling loop shutting down");
                break;
            }
            _ = interval.tick() => {
                match poll_once(&docker).await {
                    Ok(docker_table) => {
                        let config = config_rx.borrow().clone();
                        let mut table = routing_table.write().await;
                        // Remove old Docker routes
                        table.remove_by_source("docker");
                        // Insert new Docker routes, skipping if static or discovery already owns the key
                        for ((protocol, key), backend) in docker_table.entries() {
                            if let Some(existing) = table.lookup(*protocol, key) {
                                if existing.source == "static" || existing.source == "discovery" {
                                    continue;
                                }
                            }
                            table.insert(*protocol, key.clone(), backend.clone());

                            // Ensure wildcard domain pattern for HTTPS routes
                            if *protocol == ProtocolKind::Https {
                                domains::ensure_domain_for_key(
                                    key,
                                    &config.http.base_domain,
                                    config.tls.cert.as_deref().unwrap_or_default(),
                                    config.tls.key.as_deref().unwrap_or_default(),
                                );
                            }
                        }
                        let count = table.len();
                        // Update /etc/hosts with current HTTP routes
                        let base_domain = config.http.base_domain.clone();
                        crate::hosts::sync(&table, &base_domain);
                        drop(table);
                        debug!(routes = count, "Routing table updated");
                    }
                    Err(e) => {
                        error!(error = %e, "Docker polling failed");
                    }
                }
            }
            _ = config_rx.changed() => {
                let new_interval = config_rx.borrow().docker.poll_interval;
                if new_interval != interval_secs {
                    info!(old = interval_secs, new = new_interval, "Docker poll interval changed");
                    interval_secs = new_interval;
                    interval = tokio::time::interval(Duration::from_secs(interval_secs));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_label_single() {
        let json = r#"[{"protocol":"postgres","key":"appdb","port":5432}]"#;
        let routes: Vec<RouteLabel> = serde_json::from_str(json).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].protocol, ProtocolKind::Postgres);
        assert_eq!(routes[0].key, "appdb");
        assert_eq!(routes[0].port, Some(5432));
    }

    #[test]
    fn test_parse_label_multiple() {
        let json = r#"[
            {"protocol":"postgres","key":"db1"},
            {"protocol":"mysql","key":"db2","port":3307}
        ]"#;
        let routes: Vec<RouteLabel> = serde_json::from_str(json).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].protocol, ProtocolKind::Postgres);
        assert_eq!(routes[0].key, "db1");
        assert_eq!(routes[0].port, None);
        assert_eq!(routes[1].protocol, ProtocolKind::Mysql);
        assert_eq!(routes[1].key, "db2");
        assert_eq!(routes[1].port, Some(3307));
    }

    #[test]
    fn test_parse_label_smtp() {
        let json = r#"[{"protocol":"smtp","key":"mail.test.localhost"}]"#;
        let routes: Vec<RouteLabel> = serde_json::from_str(json).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].protocol, ProtocolKind::Smtp);
        assert_eq!(routes[0].key, "mail.test.localhost");
    }

    #[test]
    fn test_parse_label_invalid() {
        let json = r#"[{"protocol":"redis","key":"cache"}]"#;
        let result: std::result::Result<Vec<RouteLabel>, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_port() {
        assert_eq!(ProtocolKind::Postgres.default_port(), 5432);
        assert_eq!(ProtocolKind::Mysql.default_port(), 3306);
        assert_eq!(ProtocolKind::Smtp.default_port(), 25);
        assert_eq!(ProtocolKind::Http.default_port(), 80);
    }
}
