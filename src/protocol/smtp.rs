use std::net::SocketAddr;
use std::path::PathBuf;

use chrono::Utc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::error::Result;
use crate::protocol::ProtocolHandler;

#[derive(Debug, PartialEq)]
enum SmtpState {
    Greeting,
    Ehlo,
    MailFrom,
    RcptTo,
    Data,
    Receiving,
    Done,
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
        let mut rcpt_domain = String::new();
        let mut line_buf = String::new();

        // Send greeting
        writer
            .write_all(b"220 name-route SMTP Ready\r\n")
            .await?;

        loop {
            line_buf.clear();
            let bytes_read = reader.read_line(&mut line_buf).await?;
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
                        // Extract domain from RCPT TO
                        rcpt_domain = extract_domain(line);
                        writer.write_all(b"250 OK\r\n").await?;
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
                        // Additional recipients - update domain
                        rcpt_domain = extract_domain(line);
                        writer.write_all(b"250 OK\r\n").await?;
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
                    // Stream directly to a temp file for memory efficiency
                    let domain_dir = if rcpt_domain.is_empty() {
                        mailbox_dir.join("unknown")
                    } else {
                        mailbox_dir.join(&rcpt_domain)
                    };

                    match receive_data(
                        &mut reader,
                        &mut writer,
                        &domain_dir,
                        max_size,
                        line, // first line of data body
                        peer,
                    )
                    .await
                    {
                        Ok(true) => {
                            state = SmtpState::MailFrom;
                        }
                        Ok(false) => {
                            // Size exceeded, already sent error
                            state = SmtpState::MailFrom;
                        }
                        Err(e) => {
                            error!(peer = %peer, error = %e, "Error receiving data");
                            break;
                        }
                    }
                }
                SmtpState::Greeting | SmtpState::Done => {
                    break;
                }
            }
        }

        Ok(())
    }
}

/// Receive DATA content, streaming to a temp file.
/// Returns Ok(true) if saved successfully, Ok(false) if size exceeded.
async fn receive_data(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    domain_dir: &std::path::Path,
    max_size: usize,
    first_line: &str,
    peer: SocketAddr,
) -> Result<bool> {
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

    // Write first line if it's actual data content
    if !first_line.is_empty() {
        let data = format!("{}\r\n", first_line);
        total_size += data.len();
        tokio::io::AsyncWriteExt::write_all(&mut file, data.as_bytes()).await?;
    }

    loop {
        line_buf.clear();
        let bytes_read = reader.read_line(&mut line_buf).await?;
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
                let n = reader.read_line(&mut line_buf).await?;
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
        return Ok(false);
    }

    info!(
        peer = %peer,
        path = %filepath.display(),
        size = total_size,
        "Email saved"
    );

    writer.write_all(b"250 OK\r\n").await?;
    Ok(true)
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
