use std::net::SocketAddr;

use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::protocol::ProtocolHandler;
use crate::protocol::ProtocolKind;
use crate::proxy;
use crate::router::SharedRoutingTable;

// MySQL capability flags
const CLIENT_CONNECT_WITH_DB: u32 = 0x0000_0008;
const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;

const SERVER_CAPABILITIES: u32 =
    CLIENT_CONNECT_WITH_DB | CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | CLIENT_PLUGIN_AUTH;

pub struct MysqlHandler {
    routing_table: SharedRoutingTable,
    config_rx: watch::Receiver<Config>,
}

impl MysqlHandler {
    pub fn new(routing_table: SharedRoutingTable, config_rx: watch::Receiver<Config>) -> Self {
        Self {
            routing_table,
            config_rx,
        }
    }
}

impl ProtocolHandler for MysqlHandler {
    async fn handle_connection(&self, mut client: TcpStream, peer: SocketAddr) -> Result<()> {
        debug!(peer = %peer, "New MySQL connection");

        // Step 1: Send synthetic HandshakeV10 to client
        let challenge = generate_challenge();
        let handshake = build_handshake_v10(&challenge);
        write_mysql_packet(&mut client, 0, &handshake).await?;

        // Step 2: Read HandshakeResponse41 from client
        let (seq, response_data) = read_mysql_packet(&mut client).await?;
        if response_data.len() < 32 {
            warn!(peer = %peer, len = response_data.len(), "HandshakeResponse41 too short");
            return Err(Error::Protocol("HandshakeResponse41 too short".into()));
        }

        // Parse client response
        let client_flags = u32::from_le_bytes([
            response_data[0],
            response_data[1],
            response_data[2],
            response_data[3],
        ]);

        // Check if CLIENT_CONNECT_WITH_DB is set
        if client_flags & CLIENT_CONNECT_WITH_DB == 0 {
            info!(peer = %peer, "No database specified (CLIENT_CONNECT_WITH_DB not set)");
            let err = build_err_packet(1049, "HY000", "No database selected");
            write_mysql_packet(&mut client, seq + 1, &err).await?;
            return Ok(());
        }

        // Parse username and database from response
        // bytes[0..4]: client_flags
        // bytes[4..8]: max_packet_size
        // byte[8]: charset
        // bytes[9..32]: filler (23 zero bytes)
        // byte 32~: username\0, auth_response (length-prefixed), database\0
        let (username, database) = parse_handshake_response(&response_data)?;

        let db_name = match &database {
            Some(name) if !name.is_empty() => name.as_str(),
            _ => {
                info!(peer = %peer, "No database in handshake response");
                let err = build_err_packet(1049, "HY000", "No database selected");
                write_mysql_packet(&mut client, seq + 1, &err).await?;
                return Ok(());
            }
        };

        debug!(peer = %peer, user = %username, database = %db_name, "MySQL routing lookup");

        // Step 3: Lookup in routing table
        let table = self.routing_table.read().await;
        let backend = table.lookup(ProtocolKind::Mysql, db_name).cloned();
        drop(table);

        let backend = match backend {
            Some(b) => b,
            None => {
                info!(peer = %peer, database = %db_name, "Database not found in routing table");
                let msg = format!("Unknown database '{}'", db_name);
                let err = build_err_packet(1049, "42000", &msg);
                write_mysql_packet(&mut client, seq + 1, &err).await?;
                return Ok(());
            }
        };

        info!(
            peer = %peer,
            database = %db_name,
            backend = %backend.container_name,
            "Routing MySQL connection"
        );

        // Step 4: Connect to backend
        let config = self.config_rx.borrow().clone();
        let mut backend_stream = proxy::connect_backend(&backend, &config.backend).await?;

        // Step 5: Read real HandshakeV10 from backend
        let (backend_seq, backend_handshake) = read_mysql_packet(&mut backend_stream).await?;

        // Parse backend challenge for auth
        let backend_challenge = parse_backend_challenge(&backend_handshake);

        // Step 6: Build new HandshakeResponse41 for backend
        let new_response =
            build_handshake_response(&username, db_name, &backend_challenge);
        write_mysql_packet(&mut backend_stream, backend_seq + 1, &new_response).await?;

        // Step 7: Read backend's OK/ERR response
        let (_resp_seq, resp_data) = read_mysql_packet(&mut backend_stream).await?;

        if !resp_data.is_empty() && resp_data[0] == 0xFF {
            // ERR packet from backend
            error!(
                peer = %peer,
                database = %db_name,
                "Backend authentication failed (password may be required)"
            );
        }

        // Forward the response to client
        write_mysql_packet(&mut client, seq + 1, &resp_data).await?;

        // If it was an ERR packet, stop here
        if !resp_data.is_empty() && resp_data[0] == 0xFF {
            return Ok(());
        }

        // Step 8: Check for additional auth exchange packets
        // Some MySQL versions send AuthSwitchRequest or additional auth data
        if !resp_data.is_empty() && resp_data[0] == 0xFE {
            // AuthSwitchRequest - we need to handle the auth exchange
            // For trust/skip-grant-tables this shouldn't happen, but log it
            warn!(
                peer = %peer,
                "Backend sent AuthSwitchRequest; auth exchange not supported"
            );
            let err = build_err_packet(1045, "28000", "Authentication method not supported by proxy");
            write_mysql_packet(&mut client, seq + 2, &err).await?;
            return Ok(());
        }

        // Step 9: Bidirectional relay
        debug!(peer = %peer, "Starting MySQL bidirectional relay");
        match copy_bidirectional(&mut client, &mut backend_stream).await {
            Ok((c2b, b2c)) => {
                debug!(peer = %peer, client_to_backend = c2b, backend_to_client = b2c, "MySQL connection closed");
            }
            Err(e) => {
                debug!(peer = %peer, error = %e, "MySQL relay ended");
            }
        }

        Ok(())
    }
}

fn generate_challenge() -> [u8; 20] {
    let mut rng = rand::thread_rng();
    let mut challenge = [0u8; 20];
    rng.fill(&mut challenge);
    // Ensure no zero bytes (MySQL protocol requirement)
    for b in &mut challenge {
        if *b == 0 {
            *b = 1;
        }
    }
    challenge
}

/// Build a MySQL HandshakeV10 packet payload.
fn build_handshake_v10(challenge: &[u8; 20]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);

    // protocol_version
    buf.push(10);

    // server_version (null-terminated) — must look like "X.Y.Z-suffix"
    // so that clients (e.g. PyMySQL) can parse the major/minor/patch numbers.
    buf.extend_from_slice(b"8.0.99-name-route\0");

    // connection_id (4 bytes LE)
    buf.extend_from_slice(&1u32.to_le_bytes());

    // auth_plugin_data_part_1 (8 bytes) - first 8 bytes of challenge
    buf.extend_from_slice(&challenge[..8]);

    // filler
    buf.push(0);

    // capability_flags_lower (2 bytes LE)
    let cap_lower = (SERVER_CAPABILITIES & 0xFFFF) as u16;
    buf.extend_from_slice(&cap_lower.to_le_bytes());

    // character_set (utf8mb4 = 45)
    buf.push(45);

    // status_flags (2 bytes LE) - SERVER_STATUS_AUTOCOMMIT
    buf.extend_from_slice(&2u16.to_le_bytes());

    // capability_flags_upper (2 bytes LE)
    let cap_upper = ((SERVER_CAPABILITIES >> 16) & 0xFFFF) as u16;
    buf.extend_from_slice(&cap_upper.to_le_bytes());

    // length of auth-plugin-data (21 for mysql_native_password: 8 + 12 + 1 null)
    buf.push(21);

    // reserved (10 zero bytes)
    buf.extend_from_slice(&[0u8; 10]);

    // auth_plugin_data_part_2 (12 bytes + null terminator)
    buf.extend_from_slice(&challenge[8..20]);
    buf.push(0);

    // auth_plugin_name (null-terminated)
    buf.extend_from_slice(b"mysql_native_password\0");

    buf
}

/// Parse the HandshakeResponse41 to extract username and database.
fn parse_handshake_response(data: &[u8]) -> Result<(String, Option<String>)> {
    let client_flags = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

    // Skip: client_flags(4) + max_packet_size(4) + charset(1) + filler(23) = 32 bytes
    let mut pos = 32;

    // Username (null-terminated)
    let username_start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    let username = String::from_utf8_lossy(&data[username_start..pos]).to_string();
    pos += 1; // skip null terminator

    // Auth response (length-prefixed if CLIENT_SECURE_CONNECTION)
    if client_flags & CLIENT_SECURE_CONNECTION != 0 {
        if pos < data.len() {
            let auth_len = data[pos] as usize;
            pos += 1 + auth_len;
        }
    } else {
        // null-terminated auth string
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1; // skip null
    }

    // Database (null-terminated, if CLIENT_CONNECT_WITH_DB)
    let database = if client_flags & CLIENT_CONNECT_WITH_DB != 0 && pos < data.len() {
        let db_start = pos;
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        Some(String::from_utf8_lossy(&data[db_start..pos]).to_string())
    } else {
        None
    };

    Ok((username, database))
}

/// Parse challenge bytes from a backend HandshakeV10.
fn parse_backend_challenge(data: &[u8]) -> Vec<u8> {
    let mut challenge = Vec::with_capacity(20);

    // protocol_version(1) + server_version (find null) + connection_id(4)
    let mut pos = 1;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    pos += 1; // skip null
    pos += 4; // skip connection_id

    // auth_plugin_data_part_1 (8 bytes)
    if pos + 8 <= data.len() {
        challenge.extend_from_slice(&data[pos..pos + 8]);
    }
    pos += 8;
    pos += 1; // filler

    // skip: cap_lower(2) + charset(1) + status(2) + cap_upper(2) + auth_len(1) + reserved(10)
    pos += 2 + 1 + 2 + 2 + 1 + 10;

    // auth_plugin_data_part_2 (12 bytes)
    if pos + 12 <= data.len() {
        challenge.extend_from_slice(&data[pos..pos + 12]);
    }

    challenge
}

/// Build a HandshakeResponse41 to send to the backend.
/// Uses empty auth_data since we expect trust authentication.
fn build_handshake_response(username: &str, database: &str, _backend_challenge: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);

    let flags = CLIENT_CONNECT_WITH_DB | CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION;

    // client_flags (4 bytes LE)
    buf.extend_from_slice(&flags.to_le_bytes());

    // max_packet_size (4 bytes LE)
    buf.extend_from_slice(&(16_777_216u32).to_le_bytes());

    // charset (utf8mb4 = 45)
    buf.push(45);

    // filler (23 zero bytes)
    buf.extend_from_slice(&[0u8; 23]);

    // username (null-terminated)
    buf.extend_from_slice(username.as_bytes());
    buf.push(0);

    // auth_response (length-prefixed, empty for trust auth)
    buf.push(0); // length = 0

    // database (null-terminated)
    buf.extend_from_slice(database.as_bytes());
    buf.push(0);

    buf
}

/// Build a MySQL ERR_Packet payload.
fn build_err_packet(error_code: u16, sql_state: &str, message: &str) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header byte
    buf.push(0xFF);

    // Error code (2 bytes LE)
    buf.extend_from_slice(&error_code.to_le_bytes());

    // SQL state marker + state (if CLIENT_PROTOCOL_41)
    buf.push(b'#');
    buf.extend_from_slice(sql_state.as_bytes());

    // Error message
    buf.extend_from_slice(message.as_bytes());

    buf
}

/// Read a MySQL packet: 3-byte length + 1-byte sequence + payload.
async fn read_mysql_packet(stream: &mut TcpStream) -> Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;

    let length = (header[0] as u32) | ((header[1] as u32) << 8) | ((header[2] as u32) << 16);
    let seq = header[3];

    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload).await?;

    Ok((seq, payload))
}

/// Write a MySQL packet: 3-byte length + 1-byte sequence + payload.
async fn write_mysql_packet(stream: &mut TcpStream, seq: u8, payload: &[u8]) -> Result<()> {
    let length = payload.len() as u32;
    let header = [
        (length & 0xFF) as u8,
        ((length >> 8) & 0xFF) as u8,
        ((length >> 16) & 0xFF) as u8,
        seq,
    ];

    stream.write_all(&header).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_challenge() {
        let c = generate_challenge();
        assert_eq!(c.len(), 20);
        assert!(c.iter().all(|&b| b != 0));
    }

    #[test]
    fn test_build_handshake_v10() {
        let challenge = [1u8; 20];
        let packet = build_handshake_v10(&challenge);

        // protocol_version
        assert_eq!(packet[0], 10);
        // server_version starts with "8.0.99-name-route"
        assert_eq!(&packet[1..18], b"8.0.99-name-route");
        assert_eq!(packet[18], 0); // null terminator
    }

    #[test]
    fn test_build_err_packet() {
        let err = build_err_packet(1049, "42000", "Unknown database 'test'");
        assert_eq!(err[0], 0xFF);
        let code = u16::from_le_bytes([err[1], err[2]]);
        assert_eq!(code, 1049);
        assert_eq!(err[3], b'#');
        assert_eq!(&err[4..9], b"42000");
    }

    #[test]
    fn test_parse_handshake_response() {
        // Build a minimal HandshakeResponse41
        let flags = CLIENT_CONNECT_WITH_DB | CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes()); // client_flags
        data.extend_from_slice(&[0u8; 4]); // max_packet_size
        data.push(45); // charset
        data.extend_from_slice(&[0u8; 23]); // filler
        data.extend_from_slice(b"root\0"); // username
        data.push(0); // auth_response length = 0
        data.extend_from_slice(b"testdb\0"); // database

        let (user, db) = parse_handshake_response(&data).unwrap();
        assert_eq!(user, "root");
        assert_eq!(db, Some("testdb".to_string()));
    }

    #[test]
    fn test_parse_handshake_response_no_db() {
        let flags = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION;
        let mut data = Vec::new();
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&[0u8; 4]);
        data.push(45);
        data.extend_from_slice(&[0u8; 23]);
        data.extend_from_slice(b"root\0");
        data.push(0); // auth_response length = 0

        let (user, db) = parse_handshake_response(&data).unwrap();
        assert_eq!(user, "root");
        assert_eq!(db, None);
    }

    #[test]
    fn test_parse_backend_challenge() {
        let challenge = [42u8; 20];
        let handshake = build_handshake_v10(&challenge);
        let parsed = parse_backend_challenge(&handshake);
        assert_eq!(parsed.len(), 20);
        assert_eq!(&parsed[..8], &challenge[..8]);
        assert_eq!(&parsed[8..], &challenge[8..]);
    }

    #[test]
    fn test_build_handshake_response_for_backend() {
        let resp = build_handshake_response("admin", "myapp", &[0u8; 20]);
        // Verify flags
        let flags = u32::from_le_bytes([resp[0], resp[1], resp[2], resp[3]]);
        assert!(flags & CLIENT_CONNECT_WITH_DB != 0);
        assert!(flags & CLIENT_PROTOCOL_41 != 0);
        // Verify username is present
        let after_filler = &resp[32..];
        assert!(after_filler.starts_with(b"admin\0"));
    }
}
