use std::collections::HashMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::domains;
use crate::hosts;
use crate::protocol::{ProtocolKind, TlsMode};
use crate::router::{Backend, HealthStatus, SharedHealthMap, SharedRoutingTable};

pub const DEFAULT_MANAGEMENT_PORT: u16 = 14321;

pub fn management_port() -> u16 {
    std::env::var("NAMEROUTE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MANAGEMENT_PORT)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    AddRoute {
        protocol: ProtocolKind,
        key: String,
        backend: String,
        #[serde(default)]
        tls_mode: Option<TlsMode>,
    },
    RemoveRoute {
        protocol: ProtocolKind,
        key: String,
    },
    ListRoutes,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<RouteEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteEntry {
    pub protocol: ProtocolKind,
    pub key: String,
    pub backend: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_mode: Option<TlsMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            routes: None,
            url: None,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(msg.into()),
            routes: None,
            url: None,
        }
    }
}

/// Validate a routing key. Only allows hostname-safe characters:
/// alphanumeric, hyphens, and dots. Must start and end with alphanumeric.
/// Max length 253 (DNS label limit).
fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("routing key must not be empty".into());
    }
    if key.len() > 253 {
        return Err("routing key too long (max 253 characters)".into());
    }
    // Must match: starts with alnum, optional middle of alnum/hyphen/dot, ends with alnum
    // Single character keys (just alnum) are also valid.
    let valid = key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && key.as_bytes()[0].is_ascii_alphanumeric()
        && key.as_bytes()[key.len() - 1].is_ascii_alphanumeric()
        && !key.contains("..");
    if !valid {
        return Err(format!(
            "invalid routing key '{}': must contain only [a-zA-Z0-9.-], start/end with alphanumeric, no consecutive dots",
            key
        ));
    }
    Ok(())
}

// --- Server ---

pub async fn run_control_server(
    port: u16,
    table: SharedRoutingTable,
    health_map: SharedHealthMap,
    base_domain: String,
    tls_cert: String,
    tls_key: String,
    listener_ports: HashMap<ProtocolKind, u16>,
    cancel: CancellationToken,
) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                error!(port = port, "Management port already in use — is another daemon running?");
            } else {
                error!(error = %e, port = port, "Failed to bind management port");
            }
            return;
        }
    };

    info!(port = port, "Management server listening");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let table = table.clone();
                        let health_map = health_map.clone();
                        let base_domain = base_domain.clone();
                        let tls_cert = tls_cert.clone();
                        let tls_key = tls_key.clone();
                        let listener_ports = listener_ports.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, table, health_map, &base_domain, &tls_cert, &tls_key, &listener_ports).await {
                                warn!(error = %e, "Management connection error");
                            }
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "Management server accept error");
                    }
                }
            }
        }
    }

    info!("Management server stopped");
}

async fn handle_connection(
    stream: TcpStream,
    table: SharedRoutingTable,
    health_map: SharedHealthMap,
    base_domain: &str,
    tls_cert: &str,
    tls_key: &str,
    listener_ports: &HashMap<ProtocolKind, u16>,
) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(req, &table, &health_map, base_domain, tls_cert, tls_key, listener_ports).await,
            Err(e) => Response::error(format!("invalid request: {}", e)),
        };

        let mut json = serde_json::to_string(&response).unwrap();
        json.push('\n');
        writer.write_all(json.as_bytes()).await?;
    }

    Ok(())
}

async fn handle_request(
    req: Request,
    table: &SharedRoutingTable,
    health_map: &SharedHealthMap,
    base_domain: &str,
    tls_cert: &str,
    tls_key: &str,
    listener_ports: &HashMap<ProtocolKind, u16>,
) -> Response {
    match req {
        Request::AddRoute {
            protocol,
            key,
            backend,
            tls_mode,
        } => {
            if let Err(e) = validate_key(&key) {
                return Response::error(e);
            }

            let (host, port) = match parse_backend(&backend) {
                Ok(v) => v,
                Err(e) => return Response::error(e),
            };

            let backend_entry = Backend {
                source: "run".to_string(),
                container_name: key.clone(),
                addrs: vec![host],
                port,
                tls_mode: tls_mode.unwrap_or(TlsMode::Passthrough),
            };

            {
                let mut t = table.write().await;
                t.insert(protocol, key.clone(), backend_entry);
            }

            // Sync /etc/hosts for HTTP/HTTPS routes
            if protocol == ProtocolKind::Http || protocol == ProtocolKind::Https {
                let t = table.read().await;
                hosts::sync(&t, base_domain);
            }

            // Ensure wildcard domain pattern for HTTPS terminate mode
            if protocol == ProtocolKind::Https {
                domains::ensure_domain_for_key(&key, base_domain, tls_cert, tls_key);
            }

            let url = build_url(protocol, &key, base_domain, listener_ports);
            info!(protocol = %protocol, key = %key, backend = %backend, "Route added via control socket");
            Response { ok: true, error: None, routes: None, url }
        }
        Request::RemoveRoute { protocol, key } => {
            if let Err(e) = validate_key(&key) {
                return Response::error(e);
            }

            let removed = {
                let mut t = table.write().await;
                t.remove(protocol, &key)
            };

            if removed {
                if protocol == ProtocolKind::Http || protocol == ProtocolKind::Https {
                    let t = table.read().await;
                    hosts::sync(&t, base_domain);
                }
                info!(protocol = %protocol, key = %key, "Route removed via control socket");
                Response::ok()
            } else {
                Response::error(format!("route not found: {}:{}", protocol, key))
            }
        }
        Request::ListRoutes => {
            let t = table.read().await;
            let hm = health_map.read().await;
            let routes: Vec<RouteEntry> = t
                .entries()
                .map(|((protocol, key), backend)| {
                    let addr = backend
                        .addrs
                        .first()
                        .map(|a| format!("{}:{}", a, backend.port))
                        .unwrap_or_else(|| format!("???:{}", backend.port));
                    let tls_mode = if *protocol == ProtocolKind::Https {
                        Some(backend.tls_mode)
                    } else {
                        None
                    };
                    let health = hm.get(&(*protocol, key.clone())).map(|s| match s {
                        HealthStatus::Healthy => "healthy".to_string(),
                        HealthStatus::Unhealthy => "unhealthy".to_string(),
                    });
                    let url = build_url(*protocol, key, base_domain, listener_ports);
                    RouteEntry {
                        protocol: *protocol,
                        key: key.clone(),
                        backend: addr,
                        source: backend.source.clone(),
                        tls_mode,
                        health,
                        url,
                    }
                })
                .collect();

            Response {
                ok: true,
                error: None,
                routes: Some(routes),
                url: None,
            }
        }
    }
}

fn build_url(
    protocol: ProtocolKind,
    key: &str,
    base_domain: &str,
    listener_ports: &HashMap<ProtocolKind, u16>,
) -> Option<String> {
    match protocol {
        ProtocolKind::Http => listener_ports
            .get(&ProtocolKind::Http)
            .map(|port| format!("http://{}.{}:{}", key, base_domain, port)),
        ProtocolKind::Https => listener_ports
            .get(&ProtocolKind::Https)
            .map(|port| format!("https://{}.{}:{}", key, base_domain, port)),
        _ => None,
    }
}

fn parse_backend(s: &str) -> Result<(IpAddr, u16), String> {
    let (host, port_str) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid backend address (expected host:port): {}", s))?;

    let addr: IpAddr = host
        .parse()
        .map_err(|_| format!("invalid IP address: {}", host))?;

    let port: u16 = port_str
        .parse()
        .map_err(|_| format!("invalid port: {}", port_str))?;

    Ok((addr, port))
}

// --- Client ---

pub async fn send_request(port: u16, req: &Request) -> Result<Response, String> {
    let addr = format!("127.0.0.1:{}", port);

    let stream = TcpStream::connect(&addr)
        .await
        .map_err(|_| format!(
            "daemon is not running (failed to connect to {}). Start with: nameroute serve",
            addr
        ))?;

    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(req).map_err(|e| e.to_string())?;
    json.push('\n');
    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("failed to send request: {}", e))?;
    writer
        .shutdown()
        .await
        .map_err(|e| format!("failed to shutdown write: {}", e))?;

    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await
        .map_err(|e| format!("failed to read response: {}", e))?
        .ok_or_else(|| "empty response from daemon".to_string())?;

    serde_json::from_str(&line).map_err(|e| format!("invalid response: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_key_valid() {
        assert!(validate_key("myapp").is_ok());
        assert!(validate_key("my-app").is_ok());
        assert!(validate_key("my.app").is_ok());
        assert!(validate_key("My-App.v2").is_ok());
        assert!(validate_key("a").is_ok());
        assert!(validate_key("a1").is_ok());
        assert!(validate_key("sub.domain.app").is_ok());
    }

    #[test]
    fn test_validate_key_invalid() {
        assert!(validate_key("").is_err());
        assert!(validate_key("-app").is_err());
        assert!(validate_key("app-").is_err());
        assert!(validate_key(".app").is_err());
        assert!(validate_key("app.").is_err());
        assert!(validate_key("my app").is_err());
        assert!(validate_key("my\napp").is_err());
        assert!(validate_key("my\tapp").is_err());
        assert!(validate_key("../../etc").is_err());
        assert!(validate_key("app..test").is_err());
        assert!(validate_key("app/test").is_err());
        assert!(validate_key("evil\r\n127.0.0.1 attacker.com").is_err());
    }

    #[test]
    fn test_validate_key_length_limit() {
        let long_key = "a".repeat(253);
        assert!(validate_key(&long_key).is_ok());
        let too_long = "a".repeat(254);
        assert!(validate_key(&too_long).is_err());
    }
}
