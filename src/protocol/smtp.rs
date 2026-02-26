use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::protocol::ProtocolHandler;

/// Maximum SMTP command line length (RFC 5321: 512 bytes including CRLF).
const MAX_COMMAND_LINE: usize = 1024;
/// Maximum line length during DATA phase (generous: 100 KB per line).
const MAX_DATA_LINE: usize = 102_400;
/// Maximum SMTP session duration (10 minutes).
const SESSION_TIMEOUT: Duration = Duration::from_secs(600);
/// Maximum number of recipients per message (RFC 5321 recommends at least 100).
const MAX_RECIPIENTS: usize = 100;

#[derive(Debug, PartialEq)]
enum SmtpState {
    Ehlo,
    MailFrom,
    RcptTo,
    Data,
    Receiving,
}

pub struct SmtpHandler {
    config_rx: watch::Receiver<Config>,
}

impl SmtpHandler {
    pub fn new(config_rx: watch::Receiver<Config>) -> Self {
        Self { config_rx }
    }
}

impl ProtocolHandler for SmtpHandler {
    async fn handle_connection(&self, client: TcpStream, peer: SocketAddr) -> Result<()> {
        debug!(peer = %peer, "New SMTP connection");

        let config = self.config_rx.borrow().clone();
        let mailbox_dir = PathBuf::from(&config.smtp.mailbox_dir);
        let max_size = config.smtp.max_message_size;

        let (reader, mut writer) = client.into_split();
        let mut reader = BufReader::new(reader);
        let mut state = SmtpState::Ehlo;
        let mut rcpt_domains: Vec<String> = Vec::new();
        let mut line_buf = String::new();

        // Send greeting
        writer
            .write_all(b"220 name-route SMTP Ready\r\n")
            .await?;

        let session = async {
        loop {
            line_buf.clear();
            let bytes_read = crate::proxy::read_limited_line(&mut reader, &mut line_buf, MAX_COMMAND_LINE).await?;
            if bytes_read == 0 {
                debug!(peer = %peer, "Client disconnected");
                break;
            }

            let line = line_buf.trim_end();
            let upper = line.to_uppercase();

            match state {
                SmtpState::Ehlo => {
                    if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        let response = format!(
                            "250-name-route\r\n250-SIZE {}\r\n250 OK\r\n",
                            max_size
                        );
                        writer.write_all(response.as_bytes()).await?;
                        state = SmtpState::MailFrom;
                    } else if upper.starts_with("QUIT") {
                        writer.write_all(b"221 Bye\r\n").await?;
                        break;
                    } else {
                        writer
                            .write_all(b"503 Expected EHLO/HELO\r\n")
                            .await?;
                    }
                }
                SmtpState::MailFrom => {
                    if upper.starts_with("MAIL FROM:") {
                        let from = line.get(10..).unwrap_or("").trim();
                        rcpt_domains.clear();
                        debug!(peer = %peer, from = %from, "MAIL FROM");
                        writer.write_all(b"250 OK\r\n").await?;
                        state = SmtpState::RcptTo;
                    } else if upper.starts_with("STARTTLS") {
                        warn!(peer = %peer, "STARTTLS requested but not supported");
                        writer
                            .write_all(b"502 Not supported\r\n")
                            .await?;
                        break;
                    } else if upper.starts_with("QUIT") {
                        writer.write_all(b"221 Bye\r\n").await?;
                        break;
                    } else if upper.starts_with("RSET") {
                        writer.write_all(b"250 OK\r\n").await?;
                        // Stay in MailFrom state
                    } else {
                        writer
                            .write_all(b"503 Expected MAIL FROM\r\n")
                            .await?;
                    }
                }
                SmtpState::RcptTo => {
                    if upper.starts_with("RCPT TO:") {
                        if rcpt_domains.len() >= MAX_RECIPIENTS {
                            writer.write_all(b"452 Too many recipients\r\n").await?;
                        } else {
                            let domain = extract_domain(line);
                            if !rcpt_domains.contains(&domain) {
                                rcpt_domains.push(domain);
                            }
                            writer.write_all(b"250 OK\r\n").await?;
                        }
                        state = SmtpState::Data;
                    } else if upper.starts_with("QUIT") {
                        writer.write_all(b"221 Bye\r\n").await?;
                        break;
                    } else if upper.starts_with("RSET") {
                        writer.write_all(b"250 OK\r\n").await?;
                        state = SmtpState::MailFrom;
                    } else {
                        writer
                            .write_all(b"503 Expected RCPT TO\r\n")
                            .await?;
                    }
                }
                SmtpState::Data => {
                    if upper.starts_with("RCPT TO:") {
                        if rcpt_domains.len() >= MAX_RECIPIENTS {
                            writer.write_all(b"452 Too many recipients\r\n").await?;
                        } else {
                            let domain = extract_domain(line);
                            if !rcpt_domains.contains(&domain) {
                                rcpt_domains.push(domain);
                            }
                            writer.write_all(b"250 OK\r\n").await?;
                        }
                    } else if upper == "DATA" {
                        writer
                            .write_all(b"354 Start mail input\r\n")
                            .await?;
                        state = SmtpState::Receiving;
                    } else if upper.starts_with("QUIT") {
                        writer.write_all(b"221 Bye\r\n").await?;
                        break;
                    } else if upper.starts_with("RSET") {
                        writer.write_all(b"250 OK\r\n").await?;
                        state = SmtpState::MailFrom;
                    } else {
                        writer.write_all(b"503 Expected DATA\r\n").await?;
                    }
                }
                SmtpState::Receiving => {
                    // Receive data until \r\n.\r\n
                    // Determine target domain directories
                    let domains: Vec<String> = if rcpt_domains.is_empty() {
                        vec!["unknown".to_string()]
                    } else {
                        rcpt_domains.clone()
                    };

                    // Save to the first domain, then hardlink to others
                    let primary_dir = mailbox_dir.join(&domains[0]);

                    match receive_data(
                        &mut reader,
                        &mut writer,
                        &primary_dir,
                        max_size,
                        peer,
                    )
                    .await
                    {
                        Ok(Some(saved_path)) => {
                            // Hardlink or copy to additional domain directories
                            for domain in &domains[1..] {
                                let extra_dir = mailbox_dir.join(domain);
                                if let Err(e) = tokio::fs::create_dir_all(&extra_dir).await {
                                    warn!(domain = %domain, error = %e, "Failed to create mailbox dir");
                                    continue;
                                }
                                let dest = extra_dir.join(saved_path.file_name().unwrap_or_default());
                                if tokio::fs::hard_link(&saved_path, &dest).await.is_err() {
                                    // Fallback to copy if hardlink fails (cross-device)
                                    if let Err(e) = tokio::fs::copy(&saved_path, &dest).await {
                                        warn!(domain = %domain, error = %e, "Failed to copy email to additional domain");
                                    }
                                }
                            }
                            state = SmtpState::MailFrom;
                        }
                        Ok(None) => {
                            // Size exceeded or client disconnected, already handled
                            state = SmtpState::MailFrom;
                        }
                        Err(e) => {
                            error!(peer = %peer, error = %e, "Error receiving data");
                            break;
                        }
                    }
                }
            }
        }

        Ok::<(), Error>(())
        };

        match tokio::time::timeout(SESSION_TIMEOUT, session).await {
            Ok(result) => result?,
            Err(_) => {
                warn!(peer = %peer, "SMTP session timed out");
                let _ = writer.write_all(b"421 Session timeout\r\n").await;
            }
        }

        Ok(())
    }
}

/// Receive DATA content, streaming to a temp file.
/// Returns Ok(Some(path)) if saved successfully, Ok(None) if size exceeded or client disconnected.
async fn receive_data(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    domain_dir: &std::path::Path,
    max_size: usize,
    peer: SocketAddr,
) -> Result<Option<PathBuf>> {
    // Create directory
    tokio::fs::create_dir_all(domain_dir).await?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let uuid = Uuid::new_v4();
    let filename = format!("{}_{}.eml", timestamp, uuid);
    let filepath = domain_dir.join(&filename);

    let mut file = tokio::fs::File::create(&filepath).await?;
    let mut total_size: usize = 0;
    let mut line_buf = String::new();
    let mut size_exceeded = false;

    loop {
        line_buf.clear();
        let bytes_read = crate::proxy::read_limited_line(reader, &mut line_buf, MAX_DATA_LINE).await?;
        if bytes_read == 0 {
            // Client disconnected
            break;
        }

        // Check for end of data marker
        let trimmed = line_buf.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed == "." {
            break;
        }

        total_size += line_buf.len();
        if total_size > max_size {
            size_exceeded = true;
            // Continue reading to consume the rest of the message
            loop {
                line_buf.clear();
                let n = crate::proxy::read_limited_line(reader, &mut line_buf, MAX_DATA_LINE).await?;
                if n == 0 {
                    break;
                }
                let t = line_buf.trim_end_matches('\n').trim_end_matches('\r');
                if t == "." {
                    break;
                }
            }
            break;
        }

        // Dot-stuffing: remove leading dot if line starts with ".."
        let write_data = if line_buf.starts_with("..") {
            &line_buf[1..]
        } else {
            &line_buf
        };
        tokio::io::AsyncWriteExt::write_all(&mut file, write_data.as_bytes()).await?;
    }

    drop(file);

    if size_exceeded {
        // Remove the file
        let _ = tokio::fs::remove_file(&filepath).await;
        writer.write_all(b"554 Message too big\r\n").await?;
        warn!(peer = %peer, size = total_size, max = max_size, "Message too big");
        return Ok(None);
    }

    info!(
        peer = %peer,
        path = %filepath.display(),
        size = total_size,
        "Email saved"
    );

    writer.write_all(b"250 OK\r\n").await?;
    Ok(Some(filepath))
}

/// Extract and sanitize domain from RCPT TO:<user@domain> or similar.
/// Returns "unknown" if the domain is missing or contains unsafe characters.
fn extract_domain(rcpt_line: &str) -> String {
    // Find the part after @
    let raw = if let Some(at_pos) = rcpt_line.rfind('@') {
        let after_at = &rcpt_line[at_pos + 1..];
        // Strip trailing > and whitespace
        after_at
            .trim_end_matches('>')
            .trim()
            .to_lowercase()
    } else {
        return "unknown".to_string();
    };

    // Sanitize: only allow alphanumeric, hyphens, and dots.
    // Reject empty, path traversal (..), or any other unsafe characters.
    if raw.is_empty()
        || raw.contains("..")
        || !raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return "unknown".to_string();
    }

    // Strip leading/trailing dots
    let sanitized = raw.trim_matches('.');
    if sanitized.is_empty() {
        return "unknown".to_string();
    }

    sanitized.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("RCPT TO:<user@example.com>"),
            "example.com"
        );
        assert_eq!(
            extract_domain("RCPT TO: user@test.localhost"),
            "test.localhost"
        );
        assert_eq!(
            extract_domain("RCPT TO:<admin@Mail.Example.COM>"),
            "mail.example.com"
        );
        assert_eq!(extract_domain("RCPT TO:<user>"), "unknown");
    }

    #[test]
    fn test_extract_domain_path_traversal() {
        assert_eq!(
            extract_domain("RCPT TO:<user@../../etc>"),
            "unknown"
        );
        assert_eq!(
            extract_domain("RCPT TO:<user@..>"),
            "unknown"
        );
        assert_eq!(
            extract_domain("RCPT TO:<user@foo/../bar>"),
            "unknown"
        );
        assert_eq!(
            extract_domain("RCPT TO:<user@evil/path>"),
            "unknown"
        );
        assert_eq!(
            extract_domain("RCPT TO:<user@evil space>"),
            "unknown"
        );
        assert_eq!(
            extract_domain("RCPT TO:<user@evil\nnewline>"),
            "unknown"
        );
    }
}
