use std::io::IsTerminal;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{kill, pthread_sigmask, SigSet, SigmaskHow, Signal};
use nix::unistd::{getpgrp, tcgetpgrp, tcsetpgrp, Pid};
use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::control::{self, Request};
use crate::protocol::{ProtocolKind, TlsMode};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const KILL_GRACE: Duration = Duration::from_secs(5);
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const GROUP_POLL: Duration = Duration::from_millis(25);

#[allow(clippy::too_many_arguments)] // CLI fields intentionally map one-to-one here.
pub async fn cmd_run(
    protocol: ProtocolKind,
    key: String,
    aliases: Vec<String>,
    detect_port: bool,
    port_env: Option<String>,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    if command.is_empty() {
        return Err("no command specified".into());
    }
    let mut keys = vec![key];
    keys.extend(aliases);
    let owner = Uuid::new_v4().to_string();
    // Install handlers before registration so a signal in the small setup
    // window cannot take the parent down and strand an already-added route.
    let mut signals = Signals::install()?;
    if detect_port {
        run_detect_port(
            protocol,
            keys,
            tls_mode,
            command,
            mgmt_port,
            owner,
            &mut signals,
        )
        .await
    } else {
        run_port_mode(
            protocol,
            keys,
            port_env,
            tls_mode,
            command,
            mgmt_port,
            owner,
            &mut signals,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)] // Split by mode; avoids an opaque option bag.
async fn run_port_mode(
    protocol: ProtocolKind,
    keys: Vec<String>,
    port_env: Option<String>,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
    owner: String,
    signals: &mut Signals,
) -> Result<(), Box<dyn std::error::Error>> {
    // Keep the listener until the last possible point. TCP cannot atomically
    // transfer a listener to an arbitrary child, so a tiny bind race remains.
    // The kernel excludes ports that are currently bound by another process.
    // A route pointing at an unbound stale port is not itself a reservation.
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = reservation.local_addr()?.port();
    let backend = format!("127.0.0.1:{port}");
    register_routes_or_cleanup(protocol, &keys, &backend, tls_mode, &owner, mgmt_port).await?;
    let port_str = port.to_string();
    let args: Vec<String> = command[1..]
        .iter()
        .map(|arg| arg.replace("$PORT", &port_str))
        .collect();
    let mut cmd = base_command(&command[0]);
    cmd.args(&args)
        .env("PORT", &port_str)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(env_name) = port_env {
        cmd.env(env_name, &port_str);
    }
    // Release immediately before exec. If spawn fails, the owner-scoped routes
    // are still cleaned below.
    drop(reservation);
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            let cleanup = cleanup_routes(protocol, &keys, &owner, mgmt_port).await;
            return Err(with_cleanup_error(error.to_string(), cleanup).into());
        }
    };
    let tty = TtyGuard::give_to_child(child.id().unwrap_or_default() as i32);
    finish_child(protocol, keys, owner, mgmt_port, child, tty, signals).await
}

async fn run_detect_port(
    protocol: ProtocolKind,
    keys: Vec<String>,
    tls_mode: Option<TlsMode>,
    command: Vec<String>,
    mgmt_port: u16,
    owner: String,
    signals: &mut Signals,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = base_command(&command[0]);
    cmd.args(&command[1..])
        .env("FORCE_COLOR", "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let tty = TtyGuard::give_to_child(child.id().unwrap_or_default() as i32);
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let port_re = Regex::new(r"https?://(localhost|127\.0\.0\.1|0\.0\.0\.0):(\d+)")?;
    let (port_tx, mut port_rx) = mpsc::channel(2);
    let mut stdout_task = tokio::spawn(forward_and_detect(
        stdout,
        false,
        port_re.clone(),
        port_tx.clone(),
    ));
    let mut stderr_task = tokio::spawn(forward_and_detect(stderr, true, port_re, port_tx));
    let mut registered = false;
    // Keep one pinned wait future for the lifetime of detection. Recreating a
    // cancelled `Child::wait` future after each URL can reap the child and make
    // a later wait/stop observe "child has no pid".
    let pgid = child.id().unwrap_or_default() as i32;
    enum DetectEvent {
        Child(Result<std::process::ExitStatus, String>),
        Signal(Signal),
        RegisterError(Box<dyn std::error::Error>),
    }
    let event = {
        let child_wait = child.wait();
        tokio::pin!(child_wait);
        loop {
            tokio::select! {
                status = &mut child_wait => break DetectEvent::Child(status.map_err(|e| e.to_string())),
                _ = signals.interrupt.recv() => break DetectEvent::Signal(Signal::SIGINT),
                _ = signals.terminate.recv() => break DetectEvent::Signal(Signal::SIGTERM),
                Some(port) = port_rx.recv() => if !registered {
                    let backend = format!("127.0.0.1:{port}");
                    match register_routes_or_cleanup(protocol, &keys, &backend, tls_mode, &owner, mgmt_port).await {
                        Ok(()) => registered = true,
                        Err(error) => break DetectEvent::RegisterError(error),
                    }
                },
            }
        }
    };
    let outcome = match event {
        DetectEvent::Child(Ok(status)) => reclaim_group_after_child(&mut child, pgid, status).await,
        DetectEvent::Child(Err(error)) => Err(error),
        DetectEvent::Signal(signal) => stop_child_group(&mut child, pgid, signal).await,
        DetectEvent::RegisterError(error) => {
            let stop = stop_child_group(&mut child, pgid, Signal::SIGTERM).await;
            drop(port_rx);
            drain_output(&mut stdout_task, &mut stderr_task).await;
            drop(tty);
            return match stop {
                Ok(_) => Err(error),
                Err(stop_error) => Err(format!(
                    "{error}; child process group cleanup also failed (processes may remain): {stop_error}"
                )
                .into()),
            };
        }
    };
    // Return the terminal before control cleanup; output pipe descendants may
    // outlive the direct child, therefore this drain is explicitly bounded.
    drop(tty);
    drop(port_rx);
    drain_output(&mut stdout_task, &mut stderr_task).await;
    let cleanup = if registered {
        cleanup_routes(protocol, &keys, &owner, mgmt_port).await
    } else {
        Ok(())
    };
    complete_exit(outcome?, cleanup)
}

fn base_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    // Stay in the parent's session so an interactive controlling terminal can
    // foreground this group, while keeping signals away from the caller PG.
    cmd.process_group(0);
    cmd
}

async fn forward_and_detect<R>(reader: R, stderr: bool, re: Regex, tx: mpsc::Sender<u16>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut port_sent = false;
    while let Ok(Some(line)) = lines.next_line().await {
        if stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
        if !port_sent {
            let Some(port) = extract_port(&re, &line) else {
                continue;
            };
            // Each stream sends at most one candidate. The receiver serializes
            // them and only its first candidate can register.
            if tx.send(port).await.is_err() {
                // The child may already have exited; keep forwarding output.
                port_sent = true;
            } else {
                port_sent = true;
            }
        }
    }
}

async fn register_routes(
    protocol: ProtocolKind,
    keys: &[String],
    backend: &str,
    tls_mode: Option<TlsMode>,
    owner: &str,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = request_with_timeout(
        mgmt_port,
        Request::RunRegister {
            protocol,
            keys: keys.to_vec(),
            backend: backend.to_string(),
            tls_mode,
            owner: owner.to_string(),
        },
    )
    .await?;
    if !response.ok {
        return Err(format!(
            "failed to register route(s): {}",
            response.error.unwrap_or_default()
        )
        .into());
    }
    eprintln!("nameroute: registered {} -> {}", keys.join(", "), backend);
    if let Some(url) = response.url {
        eprintln!("nameroute: {url}");
    }
    Ok(())
}

/// A lost response can happen after a daemon committed the registration. The
/// compensating request is safe because cleanup is constrained by this UUID.
async fn register_routes_or_cleanup(
    protocol: ProtocolKind,
    keys: &[String],
    backend: &str,
    tls_mode: Option<TlsMode>,
    owner: &str,
    mgmt_port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    match register_routes(protocol, keys, backend, tls_mode, owner, mgmt_port).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let cleanup = cleanup_routes(protocol, keys, owner, mgmt_port).await;
            Err(with_cleanup_error(error.to_string(), cleanup).into())
        }
    }
}

async fn cleanup_routes(
    protocol: ProtocolKind,
    keys: &[String],
    owner: &str,
    mgmt_port: u16,
) -> Result<(), String> {
    let response = request_with_timeout(
        mgmt_port,
        Request::RunCleanup {
            protocol,
            keys: keys.to_vec(),
            owner: owner.to_string(),
        },
    )
    .await?;
    if response.ok {
        eprintln!("nameroute: routes removed");
        Ok(())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "cleanup was rejected".to_string()))
    }
}

async fn request_with_timeout(
    mgmt_port: u16,
    request: Request,
) -> Result<control::Response, String> {
    tokio::time::timeout(CONTROL_TIMEOUT, control::send_request(mgmt_port, &request))
        .await
        .map_err(|_| "management request timed out".to_string())?
}

async fn finish_child(
    protocol: ProtocolKind,
    keys: Vec<String>,
    owner: String,
    mgmt_port: u16,
    mut child: Child,
    tty: TtyGuard,
    signals: &mut Signals,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = wait_for_child_or_signal(&mut child, signals).await;
    // A TTY must always be returned before a potentially slow/failed control
    // request, including a non-zero child outcome.
    drop(tty);
    let cleanup = cleanup_routes(protocol, &keys, &owner, mgmt_port).await;
    complete_exit(outcome?, cleanup)
}

async fn drain_output(
    stdout: &mut tokio::task::JoinHandle<()>,
    stderr: &mut tokio::task::JoinHandle<()>,
) {
    if tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, async {
        let _ = (&mut *stdout).await;
        let _ = (&mut *stderr).await;
    })
    .await
    .is_err()
    {
        stdout.abort();
        stderr.abort();
        let _ = (&mut *stdout).await;
        let _ = (&mut *stderr).await;
    }
}

/// Hands an interactive terminal to the private child group. This keeps stdin
/// usable after `process_group(0)` and makes terminal Ctrl+C target the child.
/// Failures are harmless for pipes/non-controlling terminals.
struct TtyGuard(Option<Pid>);
impl TtyGuard {
    fn give_to_child(child_pgid: i32) -> Self {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() || child_pgid <= 0 {
            return Self(None);
        }
        let previous = tcgetpgrp(&stdin).ok();
        // A background `nameroute run ... &` must never seize the terminal
        // from the shell's foreground group.
        if foreground_is_ours(previous, getpgrp())
            && set_foreground(&stdin, Pid::from_raw(child_pgid)).is_ok()
        {
            // Avoid a read-before-foreground race for a just spawned child.
            let _ = kill(Pid::from_raw(-child_pgid), Signal::SIGCONT);
            Self(previous)
        } else {
            Self(None)
        }
    }
}

fn foreground_is_ours(foreground: Option<Pid>, ours: Pid) -> bool {
    foreground == Some(ours)
}
impl Drop for TtyGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            let stdin = std::io::stdin();
            let _ = set_foreground(&stdin, previous);
        }
    }
}

// `tcsetpgrp` from the temporarily-backgrounded parent would otherwise stop
// this process with SIGTTOU on some terminals, leaving the terminal owned by a
// dead child group. Block it only around the ioctl and restore the old mask.
fn set_foreground(stdin: &std::io::Stdin, pgrp: Pid) -> nix::Result<()> {
    let mut set = SigSet::empty();
    set.add(Signal::SIGTTOU);
    let mut old = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&set), Some(&mut old))?;
    let result = tcsetpgrp(stdin, pgrp);
    let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&old), None);
    result
}

struct Signals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}
impl Signals {
    fn install() -> Result<Self, std::io::Error> {
        Ok(Self {
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }
}

async fn wait_for_child_or_signal(
    child: &mut Child,
    signals: &mut Signals,
) -> Result<std::process::ExitStatus, String> {
    let pgid = child.id().ok_or_else(|| "child has no pid".to_string())? as i32;
    tokio::select! {
        status = child.wait() => reclaim_group_after_child(child, pgid, status.map_err(|e| e.to_string())?).await,
        _ = signals.interrupt.recv() => stop_child_group(child, pgid, Signal::SIGINT).await,
        _ = signals.terminate.recv() => stop_child_group(child, pgid, Signal::SIGTERM).await,
    }
}

async fn stop_child_group(
    child: &mut Child,
    pgid: i32,
    signal: Signal,
) -> Result<std::process::ExitStatus, String> {
    eprintln!("nameroute: received {signal:?}, stopping child process group...");
    send_group_signal(pgid, signal)?;
    let (status, gone) = wait_group(child, pgid, SHUTDOWN_GRACE).await;
    if !gone {
        eprintln!("nameroute: child process group did not exit; sending SIGKILL");
        send_group_signal(pgid, Signal::SIGKILL)?;
        let (after_kill, killed) = wait_group(child, pgid, KILL_GRACE).await;
        if !killed {
            return Err("child process group did not exit after SIGKILL".to_string());
        }
        return wait_direct_child(child, after_kill.or(status)).await;
    }
    wait_direct_child(child, status).await
}

async fn reclaim_group_after_child(
    child: &mut Child,
    pgid: i32,
    status: std::process::ExitStatus,
) -> Result<std::process::ExitStatus, String> {
    if !group_exists(pgid) {
        return Ok(status);
    }
    eprintln!("nameroute: child exited but descendants remain; stopping process group...");
    send_group_signal(pgid, Signal::SIGTERM)?;
    let (_, gone) = wait_group(child, pgid, SHUTDOWN_GRACE).await;
    if !gone {
        eprintln!("nameroute: descendants did not exit; sending SIGKILL");
        send_group_signal(pgid, Signal::SIGKILL)?;
        let (_, killed) = wait_group(child, pgid, KILL_GRACE).await;
        if !killed {
            return Err("child process group did not exit after SIGKILL".to_string());
        }
    }
    Ok(status)
}

fn group_exists(pgid: i32) -> bool {
    match kill(Pid::from_raw(-pgid), None) {
        Ok(()) => true,
        Err(Errno::ESRCH) => false,
        // EPERM still proves a process in the group exists.
        Err(_) => true,
    }
}

fn send_group_signal(pgid: i32, signal: Signal) -> Result<(), String> {
    match kill(Pid::from_raw(-pgid), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

async fn wait_group(
    child: &mut Child,
    pgid: i32,
    limit: Duration,
) -> (Option<std::process::ExitStatus>, bool) {
    let deadline = tokio::time::Instant::now() + limit;
    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(_) => return (status, false),
            }
        }
        if !group_exists(pgid) {
            return (status, true);
        }
        if tokio::time::Instant::now() >= deadline {
            return (status, false);
        }
        tokio::time::sleep(GROUP_POLL).await;
    }
}

async fn wait_direct_child(
    child: &mut Child,
    status: Option<std::process::ExitStatus>,
) -> Result<std::process::ExitStatus, String> {
    match status {
        Some(status) => Ok(status),
        None => tokio::time::timeout(KILL_GRACE, child.wait())
            .await
            .map_err(|_| "direct child did not exit".to_string())?
            .map_err(|e| e.to_string()),
    }
}

fn complete_exit(
    status: std::process::ExitStatus,
    cleanup: Result<(), String>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !status.success() {
        if let Err(error) = cleanup {
            eprintln!("nameroute: warning: route cleanup failed (routes may remain): {error}");
        }
        // Preserve conventional shell status for a signal-terminated child.
        std::process::exit(
            status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
        );
    }
    if let Err(error) = cleanup {
        return Err(format!("route cleanup failed (routes may remain): {error}").into());
    }
    std::process::exit(status.code().unwrap_or(1));
}

fn with_cleanup_error(original: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => original,
        Err(cleanup) => {
            format!("{original}; route cleanup also failed (routes may remain): {cleanup}")
        }
    }
}

fn extract_port(re: &Regex, line: &str) -> Option<u16> {
    re.captures(line)
        .and_then(|cap| cap.get(2))
        .and_then(|m| m.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn extracts_port() {
        let re = Regex::new(r"https?://(localhost|127\.0\.0\.1|0\.0\.0\.0):(\d+)").unwrap();
        assert_eq!(
            extract_port(&re, "ready at http://localhost:3000"),
            Some(3000)
        );
    }

    #[tokio::test]
    async fn private_process_group_reclaims_descendant_after_direct_child_exits() {
        let mut command = base_command("sh");
        command.arg("-c").arg("sleep 30 &");
        let mut child = command.spawn().unwrap();
        let pgid = child.id().unwrap() as i32;
        let status = child.wait().await.unwrap();
        assert!(
            group_exists(pgid),
            "background sleep should still own the group"
        );
        let observed = reclaim_group_after_child(&mut child, pgid, status)
            .await
            .unwrap();
        assert!(observed.success());
        assert!(!group_exists(pgid));
    }

    #[tokio::test]
    async fn signal_stops_the_entire_private_process_group() {
        let mut command = base_command("sh");
        command
            .arg("-c")
            .arg("trap 'exit 42' TERM; while :; do sleep 1; done")
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let pgid = child.id().unwrap() as i32;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = stop_child_group(&mut child, pgid, Signal::SIGTERM)
            .await
            .unwrap();
        assert_eq!(status.code(), Some(42));
        assert!(!group_exists(pgid));
    }

    #[test]
    fn cleanup_diagnostic_preserves_original_error() {
        let message = with_cleanup_error(
            "registration timeout".to_string(),
            Err("cleanup timeout".to_string()),
        );
        assert!(message.starts_with("registration timeout"));
        assert!(message.contains("cleanup timeout"));
    }

    #[test]
    fn only_foreground_parent_may_handoff_tty() {
        assert!(foreground_is_ours(
            Some(Pid::from_raw(10)),
            Pid::from_raw(10)
        ));
        assert!(!foreground_is_ours(
            Some(Pid::from_raw(11)),
            Pid::from_raw(10)
        ));
        assert!(!foreground_is_ours(None, Pid::from_raw(10)));
    }

    #[tokio::test]
    async fn lost_registration_response_still_issues_owner_cleanup() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let mut first_line = String::new();
            BufReader::new(first)
                .read_line(&mut first_line)
                .await
                .unwrap();
            assert!(first_line.contains("run_register"));
            // Model a daemon that committed but died before its response.
            let (second, _) = listener.accept().await.unwrap();
            let (read, mut write) = second.into_split();
            let mut second_line = String::new();
            BufReader::new(read)
                .read_line(&mut second_line)
                .await
                .unwrap();
            assert!(second_line.contains("run_cleanup"));
            write
                .write_all(
                    br#"{"ok":true}
"#,
                )
                .await
                .unwrap();
        });
        let keys = vec!["app".to_string()];
        let error = register_routes_or_cleanup(
            ProtocolKind::Postgres,
            &keys,
            "127.0.0.1:5432",
            None,
            "owner",
            port,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("empty response"));
        server.await.unwrap();
    }
}
