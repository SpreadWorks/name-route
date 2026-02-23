use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::error::Result;

/// Run the DNS server on the given bind address.
/// Responds to A queries with 127.0.0.1 and AAAA queries with ::1
/// for domains matching *.{base_domain} or {base_domain} itself.
/// Other domains get REFUSED. Other query types get NXDOMAIN.
pub async fn run_dns_server(
    bind: &str,
    base_domain: &str,
    cancel: CancellationToken,
) -> Result<()> {
    let socket = UdpSocket::bind(bind).await?;
    info!(bind = %bind, base_domain = %base_domain, "DNS server started");

    let base_lower = base_domain.to_lowercase();
    let mut buf = [0u8; 512];

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("DNS server shutting down");
                break;
            }
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, src)) => {
                        if len < 12 {
                            continue;
                        }
                        let request = &buf[..len];
                        match handle_query(request, &base_lower) {
                            Ok(response) => {
                                if let Err(e) = socket.send_to(&response, src).await {
                                    debug!(error = %e, peer = %src, "Failed to send DNS response");
                                }
                            }
                            Err(e) => {
                                debug!(error = %e, peer = %src, "Failed to handle DNS query");
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "DNS recv error");
                    }
                }
            }
        }
    }

    Ok(())
}

// DNS header flags
const FLAG_QR: u16 = 0x8000; // Response
const FLAG_AA: u16 = 0x0400; // Authoritative Answer
const FLAG_RD: u16 = 0x0100; // Recursion Desired (copy from request)

// RCODE values
const RCODE_OK: u16 = 0;
const RCODE_NXDOMAIN: u16 = 3;
const RCODE_REFUSED: u16 = 5;

// QTYPE values
const QTYPE_A: u16 = 1;
const QTYPE_AAAA: u16 = 28;

fn handle_query(request: &[u8], base_domain: &str) -> std::result::Result<Vec<u8>, String> {
    // Parse header
    let id = u16::from_be_bytes([request[0], request[1]]);
    let flags = u16::from_be_bytes([request[2], request[3]]);
    let qdcount = u16::from_be_bytes([request[4], request[5]]);

    if qdcount == 0 {
        return Err("No questions".to_string());
    }

    // Parse question section (first question only)
    let (qname, qname_end) = parse_qname(request, 12)?;
    if qname_end + 4 > request.len() {
        return Err("Truncated question".to_string());
    }

    let qtype = u16::from_be_bytes([request[qname_end], request[qname_end + 1]]);
    let qclass = u16::from_be_bytes([request[qname_end + 2], request[qname_end + 3]]);
    let question_end = qname_end + 4;

    // Normalize domain name
    let qname_lower = qname.to_lowercase();
    let qname_trimmed = qname_lower.trim_end_matches('.');

    // Check if domain matches base_domain or *.base_domain
    let is_target = qname_trimmed == base_domain
        || qname_trimmed.ends_with(&format!(".{}", base_domain));

    if !is_target {
        // REFUSED for non-target domains
        return Ok(build_response(
            id, flags, RCODE_REFUSED, request, 12, question_end, None,
        ));
    }

    // Check query type
    match qtype {
        QTYPE_A => {
            // Respond with 127.0.0.1
            let rdata: [u8; 4] = [127, 0, 0, 1];
            let answer = build_rr(&request[12..question_end - 4], qtype, qclass, 0, &rdata);
            Ok(build_response(
                id,
                flags,
                RCODE_OK,
                request,
                12,
                question_end,
                Some(&answer),
            ))
        }
        QTYPE_AAAA => {
            // Respond with ::1
            let rdata: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
            let answer = build_rr(&request[12..question_end - 4], qtype, qclass, 0, &rdata);
            Ok(build_response(
                id,
                flags,
                RCODE_OK,
                request,
                12,
                question_end,
                Some(&answer),
            ))
        }
        _ => {
            // NXDOMAIN for unsupported query types
            Ok(build_response(
                id, flags, RCODE_NXDOMAIN, request, 12, question_end, None,
            ))
        }
    }
}

/// Parse a DNS QNAME starting at `offset` in `packet`.
/// Returns (domain_name_string, offset_after_qname).
fn parse_qname(packet: &[u8], offset: usize) -> std::result::Result<(String, usize), String> {
    let mut labels = Vec::new();
    let mut pos = offset;

    loop {
        if pos >= packet.len() {
            return Err("QNAME truncated".to_string());
        }
        let len = packet[pos] as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        // Pointer (compression) — not expected in queries but handle gracefully
        if len & 0xC0 == 0xC0 {
            return Err("Compression pointers not supported in questions".to_string());
        }
        pos += 1;
        if pos + len > packet.len() {
            return Err("QNAME label truncated".to_string());
        }
        let label = String::from_utf8_lossy(&packet[pos..pos + len]).to_string();
        labels.push(label);
        pos += len;
    }

    Ok((labels.join("."), pos))
}

/// Build a DNS resource record.
/// qname_bytes must include the trailing null byte (root label).
fn build_rr(qname_bytes: &[u8], qtype: u16, qclass: u16, ttl: u32, rdata: &[u8]) -> Vec<u8> {
    let mut rr = Vec::new();
    // NAME (QNAME bytes already end with 0x00 root label)
    rr.extend_from_slice(qname_bytes);
    // TYPE
    rr.extend_from_slice(&qtype.to_be_bytes());
    // CLASS
    rr.extend_from_slice(&qclass.to_be_bytes());
    // TTL
    rr.extend_from_slice(&ttl.to_be_bytes());
    // RDLENGTH
    rr.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    // RDATA
    rr.extend_from_slice(rdata);
    rr
}

/// Build a complete DNS response packet.
fn build_response(
    id: u16,
    req_flags: u16,
    rcode: u16,
    request: &[u8],
    question_start: usize,
    question_end: usize,
    answer: Option<&[u8]>,
) -> Vec<u8> {
    let mut resp = Vec::new();

    // Header
    resp.extend_from_slice(&id.to_be_bytes());

    let rd = req_flags & FLAG_RD;
    let resp_flags = FLAG_QR | FLAG_AA | rd | rcode;
    resp.extend_from_slice(&resp_flags.to_be_bytes());

    // QDCOUNT = 1
    resp.extend_from_slice(&1u16.to_be_bytes());
    // ANCOUNT
    let ancount: u16 = if answer.is_some() { 1 } else { 0 };
    resp.extend_from_slice(&ancount.to_be_bytes());
    // NSCOUNT = 0
    resp.extend_from_slice(&0u16.to_be_bytes());
    // ARCOUNT = 0
    resp.extend_from_slice(&0u16.to_be_bytes());

    // Question section (copy from request)
    resp.extend_from_slice(&request[question_start..question_end]);

    // Answer section
    if let Some(answer_data) = answer {
        resp.extend_from_slice(answer_data);
    }

    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal DNS query packet for testing.
    fn build_query(domain: &str, qtype: u16) -> Vec<u8> {
        let mut pkt = Vec::new();

        // Header
        pkt.extend_from_slice(&0x1234u16.to_be_bytes()); // ID
        pkt.extend_from_slice(&0x0100u16.to_be_bytes()); // Flags: RD=1
        pkt.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        pkt.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
        pkt.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        pkt.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT

        // QNAME
        for label in domain.split('.') {
            pkt.push(label.len() as u8);
            pkt.extend_from_slice(label.as_bytes());
        }
        pkt.push(0); // root label

        // QTYPE
        pkt.extend_from_slice(&qtype.to_be_bytes());
        // QCLASS (IN)
        pkt.extend_from_slice(&1u16.to_be_bytes());

        pkt
    }

    #[test]
    fn test_a_query_matching() {
        let query = build_query("dev1.localhost", QTYPE_A);
        let response = handle_query(&query, "localhost").unwrap();

        // Check response ID matches
        assert_eq!(response[0], 0x12);
        assert_eq!(response[1], 0x34);

        // Check QR bit is set (response)
        let flags = u16::from_be_bytes([response[2], response[3]]);
        assert!(flags & FLAG_QR != 0);

        // Check RCODE = 0 (no error)
        assert_eq!(flags & 0x000F, RCODE_OK);

        // Check ANCOUNT = 1
        let ancount = u16::from_be_bytes([response[6], response[7]]);
        assert_eq!(ancount, 1);

        // Find the answer section and check RDATA contains 127.0.0.1
        // The answer is after the question section, and RDATA is at the end
        let rdata_end = response.len();
        assert_eq!(
            &response[rdata_end - 4..],
            &[127, 0, 0, 1]
        );
    }

    #[test]
    fn test_aaaa_query_matching() {
        let query = build_query("dev1.localhost", QTYPE_AAAA);
        let response = handle_query(&query, "localhost").unwrap();

        let flags = u16::from_be_bytes([response[2], response[3]]);
        assert_eq!(flags & 0x000F, RCODE_OK);

        let ancount = u16::from_be_bytes([response[6], response[7]]);
        assert_eq!(ancount, 1);

        // RDATA should be ::1
        let rdata_end = response.len();
        let expected: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(&response[rdata_end - 16..], &expected);
    }

    #[test]
    fn test_base_domain_itself() {
        let query = build_query("localhost", QTYPE_A);
        let response = handle_query(&query, "localhost").unwrap();

        let flags = u16::from_be_bytes([response[2], response[3]]);
        assert_eq!(flags & 0x000F, RCODE_OK);
    }

    #[test]
    fn test_non_matching_domain_refused() {
        let query = build_query("example.com", QTYPE_A);
        let response = handle_query(&query, "localhost").unwrap();

        let flags = u16::from_be_bytes([response[2], response[3]]);
        assert_eq!(flags & 0x000F, RCODE_REFUSED);

        // ANCOUNT should be 0
        let ancount = u16::from_be_bytes([response[6], response[7]]);
        assert_eq!(ancount, 0);
    }

    #[test]
    fn test_unsupported_qtype_nxdomain() {
        // MX query (type 15)
        let query = build_query("dev1.localhost", 15);
        let response = handle_query(&query, "localhost").unwrap();

        let flags = u16::from_be_bytes([response[2], response[3]]);
        assert_eq!(flags & 0x000F, RCODE_NXDOMAIN);
    }

    #[test]
    fn test_parse_qname() {
        let mut pkt = vec![0u8; 12]; // dummy header
        // "dev1.localhost" = \x04dev1\x09localhost\x00
        pkt.push(4);
        pkt.extend_from_slice(b"dev1");
        pkt.push(9);
        pkt.extend_from_slice(b"localhost");
        pkt.push(0);

        let (name, end) = super::parse_qname(&pkt, 12).unwrap();
        assert_eq!(name, "dev1.localhost");
        assert_eq!(end, 12 + 1 + 4 + 1 + 9 + 1);
    }

    #[test]
    fn test_rd_flag_preserved() {
        let query = build_query("dev1.localhost", QTYPE_A);
        let response = handle_query(&query, "localhost").unwrap();

        let flags = u16::from_be_bytes([response[2], response[3]]);
        // RD should be set (copied from request)
        assert!(flags & FLAG_RD != 0);
    }
}
