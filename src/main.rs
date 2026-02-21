mod config;
mod docker;
mod error;
mod listener;
mod protocol;
mod proxy;
mod router;
mod signal;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use config::Config;
use protocol::ProtocolKind;

#[derive(Parser)]
#[command(name = "name-route", about = "Local TCP L7 Router")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "/etc/name-route/config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Load configuration
    let config = Config::load(&cli.config)?;

    // Initialize tracing
    init_tracing(&config);

    info!("name-route starting");

    // Connect to Docker
    let docker = docker::connect_docker(&config).await?;

    // Initial Docker poll
    let initial_table = docker::poll_once(&docker).await?;
    info!(routes = initial_table.len(), "Initial routing table built");

    let routing_table = router::new_shared_routing_table();
    *routing_table.write().await = initial_table;

    // Config watch channel
    let (config_tx, config_rx) = watch::channel(config.clone());

    // Cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // Spawn Docker polling loop
    let docker_cancel = cancel.clone();
    let docker_table = routing_table.clone();
    let docker_config_rx = config_rx.clone();
    tokio::spawn(async move {
        docker::polling_loop(docker, docker_table, docker_config_rx, docker_cancel).await;
    });

    // Spawn signal handler
    let signal_cancel = cancel.clone();
    let config_path = cli.config.to_string_lossy().to_string();
    tokio::spawn(async move {
        signal::signal_handler(config_path, config_tx, signal_cancel).await;
    });

    // Spawn listeners for each configured protocol
    let mut listener_handles = Vec::new();

    for (name, listener_config) in &config.listeners {
        match listener_config.protocol {
            ProtocolKind::Postgres => {
                let handler = Arc::new(protocol::postgres::PostgresHandler::new(
                    routing_table.clone(),
                    config_rx.clone(),
                ));
                let lc = listener_config.clone();
                let cancel = cancel.clone();
                let name = name.clone();
                listener_handles.push(tokio::spawn(async move {
                    if let Err(e) = listener::run_listener(&lc, handler, cancel).await {
                        error!(listener = %name, error = %e, "Listener failed");
                    }
                }));
            }
            ProtocolKind::Mysql => {
                let handler = Arc::new(protocol::mysql::MysqlHandler::new(
                    routing_table.clone(),
                    config_rx.clone(),
                ));
                let lc = listener_config.clone();
                let cancel = cancel.clone();
                let name = name.clone();
                listener_handles.push(tokio::spawn(async move {
                    if let Err(e) = listener::run_listener(&lc, handler, cancel).await {
                        error!(listener = %name, error = %e, "Listener failed");
                    }
                }));
            }
            ProtocolKind::Smtp => {
                let handler = Arc::new(protocol::smtp::SmtpHandler::new(config_rx.clone()));
                let lc = listener_config.clone();
                let cancel = cancel.clone();
                let name = name.clone();
                listener_handles.push(tokio::spawn(async move {
                    if let Err(e) = listener::run_listener(&lc, handler, cancel).await {
                        error!(listener = %name, error = %e, "Listener failed");
                    }
                }));
            }
        }
    }

    // Drop privileges if configured
    if config.general.run_as_user.is_some() || config.general.run_as_group.is_some() {
        signal::drop_privileges(
            config.general.run_as_user.as_deref(),
            config.general.run_as_group.as_deref(),
        )?;
    }

    info!("name-route ready");

    // Wait for shutdown
    cancel.cancelled().await;

    info!("name-route shutting down");

    // Wait for listeners to finish
    for handle in listener_handles {
        let _ = handle.await;
    }

    info!("name-route stopped");
    Ok(())
}

fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.general.log_level));

    match config.general.log_output.as_str() {
        "file" => {
            if let Some(log_dir) = &config.general.log_dir {
                if let Err(e) = std::fs::create_dir_all(log_dir) {
                    eprintln!(
                        "WARNING: Failed to create log directory {:?}: {}. Falling back to stdout.",
                        log_dir, e
                    );
                    tracing_subscriber::fmt()
                        .with_env_filter(filter)
                        .init();
                    return;
                }
                let file_appender =
                    tracing_appender::rolling::daily(log_dir, "name-route.log");
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_writer(file_appender)
                    .with_ansi(false)
                    .init();
            } else {
                // Fallback to stdout if log_dir not set
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .init();
            }
        }
        "stderr" => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .init();
        }
        _ => {
            // "stdout" or default
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .init();
        }
    }
}
