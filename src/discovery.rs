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
    key: Option<String>,
    backend_host: Option<String>,
    #[serde(default)]
    base_domains: Option<Vec<String>>,
    #[serde(default)]
    http: Option<ProjectProtocolConfig>,
    #[serde(default)]
    https: Option<ProjectProtocolConfig>,
    #[serde(default)]
    postgres: Option<ProjectProtocolConfig>,
    #[serde(default)]
    mysql: Option<ProjectProtocolConfig>,
    #[serde(default)]
    smtp: Option<ProjectProtocolConfig>,
    #[serde(default)]
    routes: Vec<ProjectRoute>,
}

#[derive(Debug, Deserialize)]
struct ProjectProtocolConfig {
    key: Option<String>,
    backend: Option<String>,
    backend_host: Option<String>,
    port: Option<u16>,
    #[serde(default)]
    base_domains: Option<Vec<String>>,
    #[serde(default)]
    tls_mode: Option<TlsMode>,
}

#[derive(Debug, Deserialize)]
struct ProjectRoute {
    protocol: ProtocolKind,
    key: Option<String>,
    backend: String,
    #[serde(default)]
    tls_mode: Option<TlsMode>,
    #[serde(default)]
    base_domains: Option<Vec<String>>,
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
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
    let project: ProjectConfig =
        toml::from_str(&content).map_err(|e| format!("parse error: {}", e))?;

    let mut result = Vec::new();

    append_protocol_route(
        &mut result,
        ProtocolKind::Http,
        project.http.as_ref(),
        &project,
        dir_name,
    );
    append_protocol_route(
        &mut result,
        ProtocolKind::Https,
        project.https.as_ref(),
        &project,
        dir_name,
    );
    append_protocol_route(
        &mut result,
        ProtocolKind::Postgres,
        project.postgres.as_ref(),
        &project,
        dir_name,
    );
    append_protocol_route(
        &mut result,
        ProtocolKind::Mysql,
        project.mysql.as_ref(),
        &project,
        dir_name,
    );
    append_protocol_route(
        &mut result,
        ProtocolKind::Smtp,
        project.smtp.as_ref(),
        &project,
        dir_name,
    );

    for route in project.routes {
        let key = route
            .key
            .or_else(|| project.key.clone())
            .unwrap_or_else(|| dir_name.to_string());

        if let Err(e) = control::validate_key(&key) {
            warn!(key = %key, error = %e, "Invalid routing key in .nameroute.toml, skipping");
            continue;
        }

        let (addr, port) = match control::parse_backend(&route.backend) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    backend = %route.backend,
                    key = %key,
                    error = %e,
                    "Invalid backend address in .nameroute.toml, skipping"
                );
                continue;
            }
        };
        let base_domains = route
            .base_domains
            .or_else(|| project.base_domains.clone())
            .unwrap_or_default();
        if !valid_base_domains(&base_domains, &key) {
            continue;
        }

        let backend = Backend {
            source: "discovery".to_string(),
            owner: None,
            container_name: key.clone(),
            addrs: vec![addr],
            port,
            tls_mode: route.tls_mode.unwrap_or(TlsMode::Passthrough),
            base_domains,
        };

        result.push((route.protocol, key, backend));
    }

    Ok(result)
}

fn append_protocol_route(
    result: &mut Vec<(ProtocolKind, String, Backend)>,
    protocol: ProtocolKind,
    section: Option<&ProjectProtocolConfig>,
    project: &ProjectConfig,
    dir_name: &str,
) {
    let Some(section) = section else {
        return;
    };

    let backend = match &section.backend {
        Some(backend) => backend.clone(),
        None => {
            let Some(port) = section.port else {
                return;
            };
            let host = section
                .backend_host
                .clone()
                .or_else(|| project.backend_host.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string());
            format!("{}:{}", host, port)
        }
    };

    let key = section
        .key
        .clone()
        .or_else(|| project.key.clone())
        .unwrap_or_else(|| dir_name.to_string());

    if let Err(e) = control::validate_key(&key) {
        warn!(key = %key, error = %e, "Invalid routing key in .nameroute.toml, skipping");
        return;
    }

    let (addr, port) = match control::parse_backend(&backend) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                backend = %backend,
                key = %key,
                error = %e,
                "Invalid backend address in .nameroute.toml, skipping"
            );
            return;
        }
    };

    let base_domains = section
        .base_domains
        .clone()
        .or_else(|| project.base_domains.clone())
        .unwrap_or_default();
    if !valid_base_domains(&base_domains, &key) {
        return;
    }

    let backend = Backend {
        source: "discovery".to_string(),
        owner: None,
        container_name: key.clone(),
        addrs: vec![addr],
        port,
        tls_mode: section.tls_mode.unwrap_or(TlsMode::Passthrough),
        base_domains,
    };

    result.push((protocol, key, backend));
}

fn valid_base_domains(base_domains: &[String], key: &str) -> bool {
    if base_domains.iter().any(|domain| domain.trim().is_empty()) {
        warn!(key = %key, "Invalid empty base domain in .nameroute.toml, skipping");
        return false;
    }
    true
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
                // Preserve routes owned by static config, Docker, manual add,
                // or an active `run` session.
                for ((protocol, key), backend) in discovery_table.entries() {
                    if table.lookup(*protocol, key).is_some() {
                        continue;
                    }
                    table.insert(*protocol, key.clone(), backend.clone());

                    // Ensure wildcard domain pattern for HTTPS routes
                    if *protocol == ProtocolKind::Https {
                        let global_base_domains = config.http.effective_base_domains();
                        let base_domains = backend.effective_base_domains(&global_base_domains);
                        domains::ensure_domains_for_key(
                            key,
                            base_domains,
                            config.tls.cert.as_deref().unwrap_or_default(),
                            config.tls.key.as_deref().unwrap_or_default(),
                        );
                    }
                }
                let count = table.len();
                let base_domains = config.http.effective_base_domains();
                crate::hosts::sync(&table, &base_domains);
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
    fn test_parse_project_config_shorthand_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
key = "myapp"
backend_host = "127.0.0.1"
base_domains = ["localhost", "project.test"]

[http]
port = 3000

[postgres]
key = "myapp-db"
port = 5432
"#,
        )
        .unwrap();

        let routes = parse_project_config(&config_path, "ignored-dir").unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].0, ProtocolKind::Http);
        assert_eq!(routes[0].1, "myapp");
        assert_eq!(routes[0].2.port, 3000);
        assert_eq!(
            routes[0].2.base_domains,
            vec!["localhost".to_string(), "project.test".to_string()]
        );
        assert_eq!(routes[1].0, ProtocolKind::Postgres);
        assert_eq!(routes[1].1, "myapp-db");
    }

    #[test]
    fn test_parse_project_config_explicit_routes_override_shorthand() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join(".nameroute.toml");
        fs::write(
            &config_path,
            r#"
[http]
port = 3000

[[routes]]
protocol = "http"
backend = "127.0.0.1:4000"
"#,
        )
        .unwrap();

        let mut table = RoutingTable::new();
        for (protocol, key, backend) in parse_project_config(&config_path, "myapp").unwrap() {
            table.insert(protocol, key, backend);
        }

        let backend = table.lookup(ProtocolKind::Http, "myapp").unwrap();
        assert_eq!(backend.port, 4000);
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
