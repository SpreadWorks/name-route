use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::control;
use crate::domains;
use crate::protocol::{ProtocolKind, TlsMode};
use crate::router::{Backend, RoutingTable, SharedRoutingTable};

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    #[serde(default)]
    routes: Vec<ProjectRoute>,
}

#[derive(Debug, Deserialize)]
struct ProjectRoute {
    protocol: ProtocolKind,
    key: Option<String>,
    backend: String,
    #[serde(default)]
    tls_mode: Option<TlsMode>,
}

/// Expand `~` at the start of a path to `$HOME`.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

/// Perform a single discovery scan across all configured paths.
/// Returns a RoutingTable containing only the discovered routes.
pub fn poll_once(config: &Config) -> RoutingTable {
    let mut table = RoutingTable::new();

    for dir_path in &config.discovery.paths {
        let parent = expand_tilde(dir_path);
        let entries = match std::fs::read_dir(&parent) {
            Ok(e) => e,
            Err(e) => {
                warn!(path = %parent.display(), error = %e, "Failed to read discovery directory");
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "Failed to read directory entry");
                    continue;
                }
            };

            let sub_path = entry.path();
            if !sub_path.is_dir() {
                continue;
            }

            let config_file = sub_path.join(".nameroute.toml");
            if !config_file.exists() {
                continue;
            }

            let dir_name = match sub_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            match parse_project_config(&config_file, &dir_name) {
                Ok(routes) => {
                    for (protocol, key, backend) in routes {
                        let collision = table.insert(protocol, key.clone(), backend);
                        if collision {
                            warn!(
                                protocol = %protocol,
                                key = %key,
                                dir = %dir_name,
                                "Discovery route collision, overwriting previous entry"
                            );
                        }
                        debug!(
                            protocol = %protocol,
                            key = %key,
                            dir = %dir_name,
                            "Discovered route"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        file = %config_file.display(),
                        error = %e,
                        "Failed to parse .nameroute.toml"
                    );
                }
            }
        }
    }

    table
}

/// Parse a single `.nameroute.toml` and return a list of (protocol, key, Backend).
fn parse_project_config(
    path: &Path,
    dir_name: &str,
) -> Result<Vec<(ProtocolKind, String, Backend)>, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
    let project: ProjectConfig =
        toml::from_str(&content).map_err(|e| format!("parse error: {}", e))?;

    let mut result = Vec::new();

    for route in project.routes {
        let key = route
            .key
            .unwrap_or_else(|| dir_name.to_string());

        if let Err(e) = control::validate_key(&key) {
            warn!(key = %key, error = %e, "Invalid routing key in .nameroute.toml, skipping");
            continue;
        }

        let (host, port_str) = match route.backend.rsplit_once(':') {
            Some((h, p)) => (h, p),
            None => {
                warn!(
                    backend = %route.backend,
                    key = %key,
                    "Invalid backend address (expected host:port), skipping"
                );
                continue;
            }
        };

        let port: u16 = match port_str.parse() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    backend = %route.backend,
                    key = %key,
                    "Invalid port in backend address, skipping"
                );
                continue;
            }
        };

        let addr: IpAddr = match host.parse() {
            Ok(a) => a,
            Err(_) => {
                warn!(
                    backend = %route.backend,
                    key = %key,
                    "Invalid IP in backend address, skipping"
                );
                continue;
            }
        };

        let backend = Backend {
            source: "discovery".to_string(),
            container_name: key.clone(),
            addrs: vec![addr],
            port,
            tls_mode: route.tls_mode.unwrap_or(TlsMode::Passthrough),
        };

        result.push((route.protocol, key, backend));
    }

    Ok(result)
}

/// Run the discovery polling loop in the background.
pub async fn polling_loop(
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
    cancel: CancellationToken,
) {
    let mut config_rx = config_rx;
    let mut interval_secs = config_rx.borrow().discovery.poll_interval;
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Discovery polling loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let config = config_rx.borrow().clone();
                if !config.discovery.enabled || config.discovery.paths.is_empty() {
                    continue;
                }
                let discovery_table = {
                    let config = config.clone();
                    match tokio::task::spawn_blocking(move || poll_once(&config)).await {
                        Ok(t) => t,
                        Err(e) => {
                            warn!(error = %e, "Discovery poll task panicked");
                            continue;
                        }
                    }
                };
                let mut table = routing_table.write().await;
                // Remove old discovery routes
                table.remove_by_source("discovery");
                // Insert new discovery routes, skipping if static already owns the key
                for ((protocol, key), backend) in discovery_table.entries() {
                    if let Some(existing) = table.lookup(*protocol, key) {
                        if existing.source == "static" {
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
                let base_domain = config.http.base_domain.clone();
                crate::hosts::sync(&table, &base_domain);
                drop(table);
                debug!(routes = count, "Routing table updated (discovery)");
            }
            _ = config_rx.changed() => {
                let new_interval = config_rx.borrow().discovery.poll_interval;
                if new_interval != interval_secs {
                    info!(old = interval_secs, new = new_interval, "Discovery poll interval changed");
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
    use std::fs;

    #[test]
    fn test_expand_tilde() {
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            let expanded = expand_tilde("~/workspace");
            assert_eq!(expanded, PathBuf::from(&home).join("workspace"));

            let expanded = expand_tilde("~");
            assert_eq!(expanded, PathBuf::from(&home));
        }

        let no_tilde = expand_tilde("/tmp/foo");
        assert_eq!(no_tilde, PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn test_parse_project_config_basic() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
[[routes]]
protocol = "http"
backend = "127.0.0.1:3000"
"#,
        )
        .unwrap();

        let routes = parse_project_config(&config_path, "myapp").unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].0, ProtocolKind::Http);
        assert_eq!(routes[0].1, "myapp"); // key defaults to dir name
        assert_eq!(routes[0].2.port, 3000);
    }

    #[test]
    fn test_parse_project_config_explicit_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
[[routes]]
protocol = "http"
key = "api"
backend = "127.0.0.1:8000"
"#,
        )
        .unwrap();

        let routes = parse_project_config(&config_path, "myapp").unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].1, "api"); // explicit key
    }

    #[test]
    fn test_parse_project_config_multiple_routes() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
[[routes]]
protocol = "http"
backend = "127.0.0.1:3000"

[[routes]]
protocol = "postgres"
backend = "127.0.0.1:5432"
"#,
        )
        .unwrap();

        let routes = parse_project_config(&config_path, "myapp").unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].0, ProtocolKind::Http);
        assert_eq!(routes[1].0, ProtocolKind::Postgres);
    }

    #[test]
    fn test_parse_project_config_invalid_backend() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
[[routes]]
protocol = "http"
backend = "not-valid"
"#,
        )
        .unwrap();

        let routes = parse_project_config(&config_path, "myapp").unwrap();
        assert_eq!(routes.len(), 0); // skipped due to invalid backend
    }

    #[test]
    fn test_poll_once_discovers_projects() {
        let workspace = tempfile::tempdir().unwrap();

        // Create project with .nameroute.toml
        let project_dir = workspace.path().join("testapp");
        fs::create_dir(&project_dir).unwrap();
        fs::write(
            project_dir.join(".nameroute.toml"),
            r#"
[[routes]]
protocol = "http"
backend = "127.0.0.1:4000"
"#,
        )
        .unwrap();

        // Create project without .nameroute.toml (should be ignored)
        let other_dir = workspace.path().join("other");
        fs::create_dir(&other_dir).unwrap();

        let config = Config {
            discovery: crate::config::DiscoveryConfig {
                enabled: true,
                paths: vec![workspace.path().to_str().unwrap().to_string()],
                poll_interval: 3,
            },
            ..Config::default()
        };

        let table = poll_once(&config);
        assert_eq!(table.len(), 1);
        let backend = table.lookup(ProtocolKind::Http, "testapp").unwrap();
        assert_eq!(backend.source, "discovery");
        assert_eq!(backend.port, 4000);
    }
}
