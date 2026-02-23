//! Minimal HTTP server for E2E testing.
//! Returns a fixed 200 OK response with a body identifying the server.
//!
//! Usage: echo-http-server --port 9999 [--body "custom body"]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut port: u16 = 9999;
    let mut body = String::from("echo-http-server OK");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args[i].parse().expect("invalid port");
            }
            "--body" => {
                i += 1;
                body = args[i].clone();
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .unwrap_or_else(|e| panic!("Failed to bind to port {}: {}", port, e));

    eprintln!("echo-http-server listening on 127.0.0.1:{}", port);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let reader = BufReader::new(stream.try_clone().unwrap());

                // Read request line + headers until empty line
                for line in reader.lines() {
                    match line {
                        Ok(l) if l.is_empty() => break,
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}
