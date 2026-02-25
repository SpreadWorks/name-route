use std::process::Stdio;

use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::info;

use crate::control::{self, Request};
use crate::protocol::{ProtocolKind, TlsMode};

pub async fn cmd_run(
    protocol: ProtocolKind,
    key: String,
    detect_port: bool,
    port_env: Option<String>,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    if command.is_empty() {
        return Err("no command specified".into());
    }

    if detect_port {
        run_detect_port(protocol, key, tls_mode, command, mgmt_port).await
    } else {
        run_port_mode(protocol, key, port_env, tls_mode, command, mgmt_port).await
    }
}

async fn run_port_mode(
    protocol: ProtocolKind,
    key: String,
    port_env: Option<String>,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    // Try to reuse port from existing route for the same protocol+key
    let port = match find_existing_port(protocol, &key, mgmt_port).await {
        Some(p) if port_is_available(p) => {
            eprintln!("nameroute: reusing existing port {} for {}:{}", p, protocol, key);
            p
        }
        Some(p) => {
            eprintln!("nameroute: port {} is in use, allocating new port", p);
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let new_p = listener.local_addr()?.port();
            drop(listener);
            new_p
        }
        None => {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let p = listener.local_addr()?.port();
            drop(listener);
            p
        }
    };

    let backend = format!("127.0.0.1:{}", port);
    let port_str = port.to_string();

    // Register route with daemon
    let resp = control::send_request(mgmt_port, &Request::AddRoute {
        protocol,
        key: key.clone(),
        backend: backend.clone(),
        tls_mode,
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    if !resp.ok {
        return Err(format!(
            "failed to add route: {}",
            resp.error.unwrap_or_default()
        )
        .into());
    }

    eprintln!(
        "nameroute: registered {}:{} -> {} (PORT={})",
        protocol, key, backend, port
    );
    if let Some(url) = &resp.url {
        eprintln!("nameroute: {}", url);
    }

    // Substitute $PORT in command arguments
    let args: Vec<String> = command[1..]
        .iter()
        .map(|arg| arg.replace("$PORT", &port_str))
        .collect();

    // Spawn child process with PORT env var
    let mut cmd = Command::new(&command[0]);
    cmd.args(&args)
        .env("PORT", &port_str)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    // Set additional port env var if specified
    if let Some(ref env_name) = port_env {
        cmd.env(env_name, &port_str);
    }

    let mut child = cmd.spawn()?;

    // Wait for child or signal
    let exit_status = tokio::select! {
        status = child.wait() => status?,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nnameroute: received interrupt, shutting down...");
            send_signal_to_child(&child);
            child.wait().await?
        }
    };

    // Remove route
    cleanup_route(protocol, &key, mgmt_port).await;

    // Exit with child's exit code
    std::process::exit(exit_status.code().unwrap_or(1));
}

async fn run_detect_port(
    protocol: ProtocolKind,
    key: String,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    // Spawn child with piped stdout/stderr
    // Set FORCE_COLOR=1 to preserve colored output through the pipe
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .env("FORCE_COLOR", "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let port_re = Regex::new(r"https?://(localhost|127\.0\.0\.1|0\.0\.0\.0):(\d+)").unwrap();

    // Shared state for whether port has been detected
    let (port_tx, port_rx) = tokio::sync::watch::channel(false);

    // Forward stdout while scanning for port
    let stdout_key = key.clone();
    let stdout_re = port_re.clone();
    let stdout_tx = port_tx.clone();
    let stdout_tls_mode = tls_mode;
    let stdout_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            println!("{}", line);
            if !*stdout_tx.borrow() {
                if let Some(port) = extract_port(&stdout_re, &line) {
                    let _ = register_route(protocol, &stdout_key, port, stdout_tls_mode, mgmt_port).await;
                    let _ = stdout_tx.send(true);
                }
            }
        }
    });

    // Forward stderr while scanning for port
    let stderr_key = key.clone();
    let stderr_re = port_re;
    let stderr_tx = port_tx;
    let stderr_tls_mode = tls_mode;
    let stderr_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            eprintln!("{}", line);
            if !*stderr_tx.borrow() {
                if let Some(port) = extract_port(&stderr_re, &line) {
                    let _ = register_route(protocol, &stderr_key, port, stderr_tls_mode, mgmt_port).await;
                    let _ = stderr_tx.send(true);
                }
            }
        }
    });

    // Wait for child or signal
    let exit_status = tokio::select! {
        status = child.wait() => status?,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nnameroute: received interrupt, shutting down...");
            send_signal_to_child(&child);
            child.wait().await?
        }
    };

    // Wait for output forwarding to finish
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    // Remove route if it was registered
    if *port_rx.borrow() {
        cleanup_route(protocol, &key, mgmt_port).await;
    }

    std::process::exit(exit_status.code().unwrap_or(1));
}

fn extract_port(re: &Regex, line: &str) -> Option<u16> {
    re.captures(line)
        .and_then(|cap| cap.get(2))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

async fn register_route(
    protocol: ProtocolKind,
    key: &str,
    port: u16,
    tls_mode: Option<TlsMode>,
    mgmt_port: u16,
) -> Result<(), String> {
    let backend = format!("127.0.0.1:{}", port);
    eprintln!(
        "nameroute: detected port {}, registering {}:{} -> {}",
        port, protocol, key, backend
    );

    let resp = control::send_request(mgmt_port, &Request::AddRoute {
        protocol,
        key: key.to_string(),
        backend,
        tls_mode,
    })
    .await?;

    if resp.ok {
        if let Some(url) = &resp.url {
            eprintln!("nameroute: {}", url);
        }
    } else {
        eprintln!(
            "nameroute: warning: failed to add route: {}",
            resp.error.unwrap_or_default()
        );
    }

    Ok(())
}

async fn cleanup_route(protocol: ProtocolKind, key: &str, mgmt_port: u16) {
    info!(protocol = %protocol, key = %key, "Removing route");
    match control::send_request(mgmt_port, &Request::RemoveRoute {
        protocol,
        key: key.to_string(),
    })
    .await
    {
        Ok(resp) => {
            if resp.ok {
                eprintln!("nameroute: route {}:{} removed", protocol, key);
            } else {
                eprintln!(
                    "nameroute: warning: failed to remove route: {}",
                    resp.error.unwrap_or_default()
                );
            }
        }
        Err(e) => {
            eprintln!("nameroute: warning: failed to remove route: {}", e);
        }
    }
}

fn port_is_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

async fn find_existing_port(protocol: ProtocolKind, key: &str, mgmt_port: u16) -> Option<u16> {
    let resp = control::send_request(mgmt_port, &Request::ListRoutes).await.ok()?;
    let routes = resp.routes?;
    for route in routes {
        if route.protocol == protocol && route.key == key {
            // Parse port from "host:port" backend string
            let port_str = route.backend.rsplit_once(':')?.1;
            return port_str.parse().ok();
        }
    }
    None
}

fn send_signal_to_child(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // Send SIGINT (not SIGTERM) to match Ctrl+C behavior.
        // Node.js and other dev servers handle SIGINT gracefully.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGINT,
        );
    }
}
