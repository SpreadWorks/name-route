use std::path::Path;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::Config;

/// Handle SIGTERM and SIGHUP signals.
/// SIGTERM: cancel the token to trigger shutdown.
/// SIGHUP: reload config and send via watch channel.
pub async fn signal_handler(
    config_path: String,
    config_tx: watch::Sender<Config>,
    cancel: CancellationToken,
) {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm =
        signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
    let mut sighup =
        signal(SignalKind::hangup()).expect("Failed to register SIGHUP handler");

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down");
                cancel.cancel();
                break;
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, reloading configuration");
                match Config::load(Path::new(&config_path)) {
                    Ok(new_config) => {
                        // Warn if listener binds changed (requires restart)
                        let current = config_tx.borrow();
                        for (name, new_listener) in &new_config.listeners {
                            if let Some(old_listener) = current.listeners.get(name) {
                                if old_listener.bind != new_listener.bind {
                                    warn!(
                                        listener = %name,
                                        old_bind = %old_listener.bind,
                                        new_bind = %new_listener.bind,
                                        "Listener bind address changed; restart required"
                                    );
                                }
                            }
                        }
                        drop(current);

                        if config_tx.send(new_config).is_err() {
                            error!("Failed to broadcast new config (no receivers)");
                        } else {
                            info!("Configuration reloaded successfully");
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to reload configuration, keeping current");
                    }
                }
            }
            _ = cancel.cancelled() => {
                break;
            }
        }
    }
}

/// Drop privileges to the specified user/group.
pub fn drop_privileges(user: Option<&str>, group: Option<&str>) -> crate::error::Result<()> {
    use nix::unistd::{setgid, setuid};

    if let Some(group_name) = group {
        let gid = nix::unistd::Group::from_name(group_name)
            .map_err(|e| crate::error::Error::Config(format!("Failed to lookup group '{}': {}", group_name, e)))?
            .ok_or_else(|| crate::error::Error::Config(format!("Group '{}' not found", group_name)))?
            .gid;
        setgid(gid).map_err(|e| {
            crate::error::Error::Config(format!("Failed to setgid to '{}': {}", group_name, e))
        })?;
        info!(group = %group_name, gid = %gid, "Dropped group privileges");
    }

    if let Some(user_name) = user {
        let uid = nix::unistd::User::from_name(user_name)
            .map_err(|e| crate::error::Error::Config(format!("Failed to lookup user '{}': {}", user_name, e)))?
            .ok_or_else(|| crate::error::Error::Config(format!("User '{}' not found", user_name)))?
            .uid;
        setuid(uid).map_err(|e| {
            crate::error::Error::Config(format!("Failed to setuid to '{}': {}", user_name, e))
        })?;
        info!(user = %user_name, uid = %uid, "Dropped user privileges");
    }

    Ok(())
}
