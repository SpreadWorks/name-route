use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::protocol::ProtocolHandler;
use crate::protocol::ProtocolKind;
use crate::proxy;
use crate::router::SharedRoutingTable;

pub struct PostgresHandler {
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
}

impl PostgresHandler {
    pub fn new(routing_table: SharedRoutingTable, config_rx: watch::Receiver<Config>) -> Self {
        Self {
            routing_table,
            config_rx,
        }
    }
}

impl ProtocolHandler for PostgresHandler {
    async fn handle_connection(&self, mut client: TcpStream, peer: SocketAddr) -> Result<()> {
        debug!(peer = %peer, "New PostgreSQL connection");

        let config = self.config_rx.borrow().clone();
        let handshake_timeout = Duration::from_secs(config.backend.idle_timeout);

        const MAX_SSL_REQUESTS: usize = 3;
        let startup_read = async {
            let mut ssl_request_count = 0usize;
            loop {
                // Read message length (4 bytes, big-endian)
                let msg_len = client.read_u32().await? as usize;
                if !(8..=10240).contains(&msg_len) {
                    warn!(peer = %peer, len = msg_len, "Invalid startup message length");
                    return Err::<_, Error>(Error::Protocol("Invalid startup message length".into()));
                }

                // Read the rest of the message
                let mut buf = vec![0u8; msg_len - 4];
                client.read_exact(&mut buf).await?;

                // First 4 bytes after length are the version/code
                let code = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);

                match code {
                    // SSLRequest (80877103) — decline and wait for next message
                    0x04D2_162F => {
                        ssl_request_count += 1;
                        if ssl_request_count > MAX_SSL_REQUESTS {
                            warn!(peer = %peer, "Too many SSLRequest messages");
                            return Err::<_, Error>(Error::Protocol("Too many SSLRequest messages".into()));
                        }
                        debug!(peer = %peer, "SSL request received, declining");
                        client.write_all(b"N").await?;
                        continue;
                    }
                    // CancelRequest (80877102)
                    0x04D2_162E => {
                        debug!(peer = %peer, "Cancel request received, ignoring");
                        return Ok(None);
                    }
                    // Version 3.0 StartupMessage (196608 = 0x00030000)
                    0x0003_0000 => {
                        return Ok(Some((msg_len, buf)));
                    }
                    _ => {
                        warn!(peer = %peer, code, "Unknown startup message code");
                        return Err(Error::Protocol(format!("Unknown startup code: {}", code)));
                    }
                }
            }
        };

        let (msg_len, buf) = match tokio::time::timeout(handshake_timeout, startup_read).await {
            Ok(Ok(Some((len, buf)))) => (len, buf),
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                warn!(peer = %peer, "PostgreSQL handshake timed out");
                return Ok(());
            }
        };

        // Parse key=value\0 pairs from buf[4..]
        let params = parse_startup_params(&buf[4..]);
        let database = params
            .iter()
            .find(|(k, _)| k == "database")
            .map(|(_, v)| v.as_str());

        let db_name = match database {
            Some(name) if !name.is_empty() => name,
            _ => {
                info!(peer = %peer, "No database specified");
                let err_msg = build_error_response("08004", "No database specified in connection");
                client.write_all(&err_msg).await?;
                return Ok(());
            }
        };

        debug!(peer = %peer, database = %db_name, "PostgreSQL routing lookup");

        // Lookup in routing table
        let table = self.routing_table.read().await;
        let backend = table.lookup(ProtocolKind::Postgres, db_name).cloned();
        drop(table);

        let backend = match backend {
            Some(b) => b,
            None => {
                info!(peer = %peer, database = %db_name, "Database not found in routing table");
                let msg = format!("database \"{}\" does not exist", db_name);
                let err_msg = build_error_response("3D000", &msg);
                client.write_all(&err_msg).await?;
                return Ok(());
            }
        };

        info!(
            peer = %peer,
            database = %db_name,
            backend = %backend.container_name,
            "Routing PostgreSQL connection"
        );

        // Reconstruct the full startup message to forward
        let mut startup_msg = Vec::with_capacity(msg_len);
        startup_msg.extend_from_slice(&(msg_len as u32).to_be_bytes());
        startup_msg.extend_from_slice(&buf);

        let config = self.config_rx.borrow().clone();
        proxy::connect_and_relay(client, &backend, &config.backend, &startup_msg, peer).await
    }
}

fn parse_startup_params(data: &[u8]) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut i = 0;

    while i < data.len() {
        // End of parameters
        if data[i] == 0 {
            break;
        }

        // Read key
        let key_start = i;
        while i < data.len() && data[i] != 0 {
            i += 1;
        }
        if i >= data.len() {
            break;
        }
        let key = String::from_utf8_lossy(&data[key_start..i]).to_string();
        i += 1; // skip null terminator

        // Read value
        let val_start = i;
        while i < data.len() && data[i] != 0 {
            i += 1;
        }
        if i >= data.len() && val_start == i {
            break;
        }
        let val = String::from_utf8_lossy(&data[val_start..i]).to_string();
        if i < data.len() {
            i += 1; // skip null terminator
        }

        params.push((key, val));
    }

    params
}

/// Build a PostgreSQL ErrorResponse message.
/// Format: 'E' | Int32(len) | 'S' "FATAL\0" | 'V' "FATAL\0" | 'C' "<code>\0" | 'M' "<msg>\0" | \0
fn build_error_response(code: &str, message: &str) -> Vec<u8> {
    let mut body = Vec::new();

    // Severity
    body.push(b'S');
    body.extend_from_slice(b"FATAL\0");

    // Severity (non-localized)
    body.push(b'V');
    body.extend_from_slice(b"FATAL\0");

    // Code
    body.push(b'C');
    body.extend_from_slice(code.as_bytes());
    body.push(0);

    // Message
    body.push(b'M');
    body.extend_from_slice(message.as_bytes());
    body.push(0);

    // Terminator
    body.push(0);

    let len = (body.len() + 4) as u32; // +4 for the length field itself
    let mut msg = Vec::with_capacity(1 + 4 + body.len());
    msg.push(b'E');
    msg.extend_from_slice(&len.to_be_bytes());
    msg.extend_from_slice(&body);

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_startup_params() {
        // "user\0postgres\0database\0mydb\0\0"
        let data = b"user\0postgres\0database\0mydb\0\0";
        let params = parse_startup_params(data);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("user".to_string(), "postgres".to_string()));
        assert_eq!(params[1], ("database".to_string(), "mydb".to_string()));
    }

    #[test]
    fn test_parse_startup_params_empty() {
        let data = b"\0";
        let params = parse_startup_params(data);
        assert!(params.is_empty());
    }

    #[test]
    fn test_build_error_response() {
        let msg = build_error_response("3D000", "database \"test\" does not exist");
        assert_eq!(msg[0], b'E');
        // Verify it contains the code and message
        let body = &msg[5..]; // skip 'E' and 4-byte length
        assert!(body.windows(5).any(|w| w == b"3D000"));
        assert!(body
            .windows(10)
            .any(|w| w == b"does not e"));
    }

    #[test]
    fn test_parse_startup_params_with_extra_fields() {
        let data = b"user\0admin\0database\0appdb\0client_encoding\0UTF8\0\0";
        let params = parse_startup_params(data);
        assert_eq!(params.len(), 3);
        assert_eq!(params[0].0, "user");
        assert_eq!(params[1].0, "database");
        assert_eq!(params[1].1, "appdb");
        assert_eq!(params[2].0, "client_encoding");
    }
}
