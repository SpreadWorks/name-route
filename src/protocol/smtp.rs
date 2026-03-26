use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RcptTarget {
    domain: String,
    local: String,
}

struct SavedMessage {
    eml_path: PathBuf,
    txt_path: PathBuf,
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
        let mut rcpt_targets: Vec<RcptTarget> = Vec::new();
        let mut from_file_label = "unknown".to_string();
        let mut line_buf = String::new();

        // Send greeting
        writer.write_all(b"220 name-route SMTP Ready\r\n").await?;

        let session = async {
            loop {
                line_buf.clear();
                let bytes_read =
                    crate::proxy::read_limited_line(&mut reader, &mut line_buf, MAX_COMMAND_LINE)
                        .await?;
                if bytes_read == 0 {
                    debug!(peer = %peer, "Client disconnected");
                    break;
                }

                let line = line_buf.trim_end();
                let upper = line.to_uppercase();

                match state {
                    SmtpState::Ehlo => {
                        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                            let response =
                                format!("250-name-route\r\n250-SIZE {}\r\n250 OK\r\n", max_size);
                            writer.write_all(response.as_bytes()).await?;
                            state = SmtpState::MailFrom;
                        } else if upper.starts_with("QUIT") {
                            writer.write_all(b"221 Bye\r\n").await?;
                            break;
                        } else {
                            writer.write_all(b"503 Expected EHLO/HELO\r\n").await?;
                        }
                    }
                    SmtpState::MailFrom => {
                        if upper.starts_with("MAIL FROM:") {
                            let from = line.get(10..).unwrap_or("").trim();
                            from_file_label =
                                sanitize_mail_for_filename(parse_mail_from(from).as_deref());
                            rcpt_targets.clear();
                            debug!(peer = %peer, from = %from, "MAIL FROM");
                            writer.write_all(b"250 OK\r\n").await?;
                            state = SmtpState::RcptTo;
                        } else if upper.starts_with("STARTTLS") {
                            warn!(peer = %peer, "STARTTLS requested but not supported");
                            writer.write_all(b"502 Not supported\r\n").await?;
                            break;
                        } else if upper.starts_with("QUIT") {
                            writer.write_all(b"221 Bye\r\n").await?;
                            break;
                        } else if upper.starts_with("RSET") {
                            writer.write_all(b"250 OK\r\n").await?;
                            from_file_label = "unknown".to_string();
                            // Stay in MailFrom state
                        } else {
                            writer.write_all(b"503 Expected MAIL FROM\r\n").await?;
                        }
                    }
                    SmtpState::RcptTo => {
                        if upper.starts_with("RCPT TO:") {
                            if rcpt_targets.len() >= MAX_RECIPIENTS {
                                writer.write_all(b"452 Too many recipients\r\n").await?;
                            } else {
                                let target = extract_rcpt_target(line);
                                if !rcpt_targets.contains(&target) {
                                    rcpt_targets.push(target);
                                }
                                writer.write_all(b"250 OK\r\n").await?;
                            }
                            state = SmtpState::Data;
                        } else if upper.starts_with("QUIT") {
                            writer.write_all(b"221 Bye\r\n").await?;
                            break;
                        } else if upper.starts_with("RSET") {
                            writer.write_all(b"250 OK\r\n").await?;
                            from_file_label = "unknown".to_string();
                            state = SmtpState::MailFrom;
                        } else {
                            writer.write_all(b"503 Expected RCPT TO\r\n").await?;
                        }
                    }
                    SmtpState::Data => {
                        if upper.starts_with("RCPT TO:") {
                            if rcpt_targets.len() >= MAX_RECIPIENTS {
                                writer.write_all(b"452 Too many recipients\r\n").await?;
                            } else {
                                let target = extract_rcpt_target(line);
                                if !rcpt_targets.contains(&target) {
                                    rcpt_targets.push(target);
                                }
                                writer.write_all(b"250 OK\r\n").await?;
                            }
                        } else if upper == "DATA" {
                            writer.write_all(b"354 Start mail input\r\n").await?;
                            state = SmtpState::Receiving;
                        } else if upper.starts_with("QUIT") {
                            writer.write_all(b"221 Bye\r\n").await?;
                            break;
                        } else if upper.starts_with("RSET") {
                            writer.write_all(b"250 OK\r\n").await?;
                            from_file_label = "unknown".to_string();
                            state = SmtpState::MailFrom;
                        } else {
                            writer.write_all(b"503 Expected DATA\r\n").await?;
                        }
                    }
                    SmtpState::Receiving => {
                        // Receive data until \r\n.\r\n
                        let targets: Vec<RcptTarget> = if rcpt_targets.is_empty() {
                            vec![RcptTarget {
                                domain: "unknown".to_string(),
                                local: "unknown".to_string(),
                            }]
                        } else {
                            rcpt_targets.clone()
                        };

                        // Save to the first recipient mailbox, then hardlink/copy to others
                        let primary_dir =
                            mailbox_dir.join(&targets[0].domain).join(&targets[0].local);
                        let message_key = build_message_key(&from_file_label);

                        match receive_data(
                            &mut reader,
                            &mut writer,
                            &primary_dir,
                            max_size,
                            peer,
                            &message_key,
                        )
                        .await
                        {
                            Ok(Some(saved)) => {
                                for target in &targets[1..] {
                                    let extra_dir =
                                        mailbox_dir.join(&target.domain).join(&target.local);
                                    if let Err(e) = tokio::fs::create_dir_all(&extra_dir).await {
                                        warn!(domain = %target.domain, local = %target.local, error = %e, "Failed to create mailbox dir");
                                        continue;
                                    }

                                    let eml_dest = extra_dir
                                        .join(saved.eml_path.file_name().unwrap_or_default());
                                    if tokio::fs::hard_link(&saved.eml_path, &eml_dest)
                                        .await
                                        .is_err()
                                    {
                                        if let Err(e) =
                                            tokio::fs::copy(&saved.eml_path, &eml_dest).await
                                        {
                                            warn!(domain = %target.domain, local = %target.local, error = %e, "Failed to copy eml to additional mailbox");
                                        }
                                    }

                                    let txt_dest = extra_dir
                                        .join(saved.txt_path.file_name().unwrap_or_default());
                                    if tokio::fs::hard_link(&saved.txt_path, &txt_dest)
                                        .await
                                        .is_err()
                                    {
                                        if let Err(e) =
                                            tokio::fs::copy(&saved.txt_path, &txt_dest).await
                                        {
                                            warn!(domain = %target.domain, local = %target.local, error = %e, "Failed to copy txt to additional mailbox");
                                        }
                                    }
                                }
                                from_file_label = "unknown".to_string();
                                state = SmtpState::MailFrom;
                            }
                            Ok(None) => {
                                from_file_label = "unknown".to_string();
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

/// Receive DATA content, streaming to a file.
/// Returns Ok(Some(path)) if saved successfully, Ok(None) if size exceeded or client disconnected.
async fn receive_data(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    mailbox_dir: &Path,
    max_size: usize,
    peer: SocketAddr,
    message_key: &str,
) -> Result<Option<SavedMessage>> {
    tokio::fs::create_dir_all(mailbox_dir).await?;

    let eml_filename = format!("{}.eml", message_key);
    let txt_filename = format!("{}.txt", message_key);
    let eml_path = mailbox_dir.join(eml_filename);
    let txt_path = mailbox_dir.join(txt_filename);

    let mut file = tokio::fs::File::create(&eml_path).await?;
    let mut total_size: usize = 0;
    let mut line_buf = String::new();
    let mut size_exceeded = false;

    loop {
        line_buf.clear();
        let bytes_read =
            crate::proxy::read_limited_line(reader, &mut line_buf, MAX_DATA_LINE).await?;
        if bytes_read == 0 {
            break;
        }

        let trimmed = line_buf.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed == "." {
            break;
        }

        total_size += line_buf.len();
        if total_size > max_size {
            size_exceeded = true;
            loop {
                line_buf.clear();
                let n =
                    crate::proxy::read_limited_line(reader, &mut line_buf, MAX_DATA_LINE).await?;
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

        let write_data = if line_buf.starts_with("..") {
            &line_buf[1..]
        } else {
            &line_buf
        };
        tokio::io::AsyncWriteExt::write_all(&mut file, write_data.as_bytes()).await?;
    }

    drop(file);

    if size_exceeded {
        let _ = tokio::fs::remove_file(&eml_path).await;
        writer.write_all(b"554 Message too big\r\n").await?;
        warn!(peer = %peer, size = total_size, max = max_size, "Message too big");
        return Ok(None);
    }

    let raw = tokio::fs::read(&eml_path).await?;
    let preview = build_preview_text(&raw);
    tokio::fs::write(&txt_path, preview).await?;

    info!(
        peer = %peer,
        eml = %eml_path.display(),
        txt = %txt_path.display(),
        size = total_size,
        "Email saved"
    );

    writer.write_all(b"250 OK\r\n").await?;
    Ok(Some(SavedMessage { eml_path, txt_path }))
}

fn build_message_key(from_file_label: &str) -> String {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let short_id = Uuid::new_v4().simple().to_string();
    let short_id = &short_id[..8];
    format!("{}_{}_{}", timestamp, from_file_label, short_id)
}

fn parse_mail_from(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = trimmed.split_whitespace().next().unwrap_or("").trim();
    if path == "<>" {
        return None;
    }

    let normalized = path.trim_start_matches('<').trim_end_matches('>').trim();
    if normalized.is_empty() {
        return None;
    }

    if normalized.chars().any(|c| c == '\r' || c == '\n') {
        return None;
    }

    Some(normalized.to_string())
}

fn sanitize_mail_for_filename(mail: Option<&str>) -> String {
    let Some(mail) = mail else {
        return "unknown".to_string();
    };

    let out: String = mail
        .chars()
        .map(|c| match c {
            '@' | '.' => '-',
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect();

    let out = out.trim_matches('-');
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out.to_lowercase()
    }
}

fn extract_rcpt_target(rcpt_line: &str) -> RcptTarget {
    let Some(at_pos) = rcpt_line.rfind('@') else {
        return RcptTarget {
            domain: "unknown".to_string(),
            local: "unknown".to_string(),
        };
    };

    let before_at = &rcpt_line[..at_pos];
    let after_at = &rcpt_line[at_pos + 1..];

    let local_raw = before_at
        .trim()
        .trim_start_matches("RCPT TO:")
        .trim()
        .trim_start_matches('<')
        .trim();

    let domain_raw = after_at.trim_end_matches('>').trim();

    let local = sanitize_mailbox_component(local_raw, true);
    let domain = sanitize_mailbox_component(domain_raw, true);

    RcptTarget { domain, local }
}

fn sanitize_mailbox_component(input: &str, allow_dot: bool) -> String {
    let lowered = input.trim().to_lowercase();
    if lowered.is_empty() {
        return "unknown".to_string();
    }

    let mapped: String = lowered
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '-' | '_' => c,
            '.' if allow_dot => '.',
            '+' => '+',
            _ => '-',
        })
        .collect();

    let cleaned = mapped.trim_matches('-').trim_matches('.');
    if cleaned.is_empty()
        || cleaned.contains("..")
        || cleaned.contains('/')
        || cleaned.contains('\\')
    {
        "unknown".to_string()
    } else {
        cleaned.to_string()
    }
}

fn build_preview_text(raw_eml: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw_eml);
    let (header_part, body_part) = split_headers_and_body(&text);

    let content_type = header_value(header_part, "Content-Type").unwrap_or("(missing)");
    let from = header_value(header_part, "From").unwrap_or("(missing)");
    let to = header_value(header_part, "To").unwrap_or("(missing)");
    let cc = header_value(header_part, "Cc").unwrap_or("(missing)");
    let subject = header_value(header_part, "Subject").unwrap_or("(missing)");
    let preview_body = extract_preview_body(header_part, body_part);

    let attachments = extract_attachment_names(header_part, body_part);
    let mut out = String::new();
    out.push_str(&format!("content-type: {}\n", content_type));
    out.push_str(&format!("from: {}\n", from));
    out.push_str(&format!("to: {}\n", to));
    out.push_str(&format!("cc: {}\n", cc));
    out.push_str(&format!("subject: {}\n", subject));
    out.push_str("body:\n");
    out.push_str(&preview_body);
    if !preview_body.ends_with('\n') {
        out.push('\n');
    }
    if attachments.is_empty() {
        out.push_str("attachments: (none)\n");
    } else {
        out.push_str("attachments:\n");
        for a in attachments {
            out.push_str(&format!("- {}\n", a));
        }
    }
    out
}

fn extract_preview_body(headers: &str, body: &str) -> String {
    let content_type = header_value(headers, "Content-Type")
        .unwrap_or("")
        .to_lowercase();
    if !content_type.starts_with("multipart/") {
        return body.to_string();
    }

    let Some(boundary) = extract_boundary(headers) else {
        return "(no previewable text part)".to_string();
    };

    let mut html_candidate: Option<String> = None;
    for part in body.split(&format!("--{}", boundary)) {
        if part.trim().is_empty() || part.trim_start().starts_with("--") {
            continue;
        }
        let (part_headers, part_body) = split_headers_and_body(part);
        let part_content_type = header_value(part_headers, "Content-Type")
            .unwrap_or("")
            .to_lowercase();
        if part_content_type.starts_with("text/plain") {
            return part_body
                .trim_start_matches("\r\n")
                .trim_start_matches('\n')
                .to_string();
        }
        if html_candidate.is_none() && part_content_type.starts_with("text/html") {
            html_candidate = Some(
                part_body
                    .trim_start_matches("\r\n")
                    .trim_start_matches('\n')
                    .to_string(),
            );
        }
    }

    html_candidate.unwrap_or_else(|| "(no previewable text part)".to_string())
}

fn split_headers_and_body(raw: &str) -> (&str, &str) {
    if let Some(pos) = raw.find("\r\n\r\n") {
        let head = &raw[..pos];
        let body = &raw[pos + 4..];
        (head, body)
    } else if let Some(pos) = raw.find("\n\n") {
        let head = &raw[..pos];
        let body = &raw[pos + 2..];
        (head, body)
    } else {
        (raw, "")
    }
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim());
            }
        }
    }
    None
}

fn extract_attachment_names(headers: &str, body: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(boundary) = extract_boundary(headers) {
        for part in body.split(&format!("--{}", boundary)) {
            if part.trim().is_empty() || part.trim_start().starts_with("--") {
                continue;
            }
            if let Some(name) = extract_filename_from_part(part) {
                names.push(name);
            }
        }
    }
    names
}

fn extract_boundary(headers: &str) -> Option<String> {
    let ct = header_value(headers, "Content-Type")?;
    let lower = ct.to_lowercase();
    let key = "boundary=";
    let idx = lower.find(key)?;
    let value = ct[idx + key.len()..].trim();
    let value = value.trim_matches('"');
    let value = value.split(';').next().unwrap_or(value).trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn extract_filename_from_part(part: &str) -> Option<String> {
    let (head, _) = split_headers_and_body(part);

    for h in ["Content-Disposition", "Content-Type"] {
        if let Some(v) = header_value(head, h) {
            let lower = v.to_lowercase();
            if let Some(idx) = lower.find("filename=") {
                let raw = v[idx + 9..].split(';').next().unwrap_or("").trim();
                let raw = raw.trim_matches('"');
                if raw.is_empty() {
                    return Some("unnamed".to_string());
                }
                return Some(raw.to_string());
            }
            if let Some(idx) = lower.find("name=") {
                let raw = v[idx + 5..].split(';').next().unwrap_or("").trim();
                let raw = raw.trim_matches('"');
                if raw.is_empty() {
                    return Some("unnamed".to_string());
                }
                return Some(raw.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rcpt_target() {
        let t = extract_rcpt_target("RCPT TO:<user@example.com>");
        assert_eq!(t.domain, "example.com");
        assert_eq!(t.local, "user");

        let t = extract_rcpt_target("RCPT TO:<Crank.In+video@zitto.jp>");
        assert_eq!(t.domain, "zitto.jp");
        assert_eq!(t.local, "crank.in+video");
    }

    #[test]
    fn test_extract_rcpt_target_invalid() {
        let t = extract_rcpt_target("RCPT TO:<user@../../etc>");
        assert_eq!(t.domain, "unknown");

        let t = extract_rcpt_target("RCPT TO:<>");
        assert_eq!(t.domain, "unknown");
        assert_eq!(t.local, "unknown");
    }

    #[test]
    fn test_parse_mail_from_and_sanitize() {
        assert_eq!(
            parse_mail_from("<aaa@spreadworks.co.jp> SIZE=123"),
            Some("aaa@spreadworks.co.jp".to_string())
        );
        assert_eq!(
            sanitize_mail_for_filename(Some("aaa@spreadworks.co.jp")),
            "aaa-spreadworks-co-jp"
        );
        assert_eq!(sanitize_mail_for_filename(None), "unknown");
        assert_eq!(parse_mail_from("<>"), None);
    }

    #[test]
    fn test_build_preview_text_with_attachment() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"b\"\r\n",
            "From: aaa@example.com\r\n",
            "To: user@example.com\r\n",
            "Subject: hello\r\n",
            "\r\n",
            "--b\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "hello\r\n",
            "--b\r\n",
            "Content-Type: application/pdf; name=\"a.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"a.pdf\"\r\n\r\n",
            "JVBERi0x\r\n",
            "--b--\r\n"
        )
        .as_bytes();
        let preview = build_preview_text(raw);
        assert!(preview.contains("content-type:"));
        assert!(preview.contains("from: aaa@example.com"));
        assert!(preview.contains("attachments:"));
        assert!(preview.contains("- a.pdf"));
    }
}
