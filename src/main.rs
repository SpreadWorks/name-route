mod config;
mod control;
mod discovery;
mod dns;
mod docker;
mod error;
mod hosts;
mod listener;
mod protocol;
mod proxy;
mod router;
mod run;
mod signal;
mod tls;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use config::Config;
use protocol::{ProtocolKind, TlsMode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BANNER: &str = r#"
  _ __   __ _ _ __ ___   ___       _ __ ___  _   _| |_ ___
 | '_ \ / _` | '_ ` _ \ / _ \_____| '__/ _ \| | | | __/ _ \
 | | | | (_| | | | | | |  __/_____| | | (_) | |_| | ||  __/
 |_| |_|\__,_|_| |_| |_|\___|     |_|  \___/ \__,_|\__\___|
"#;

#[derive(Parser)]
#[command(name = "nameroute", version, about = "Local TCP L7 Router")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as a daemon (for systemd/launchd)
    Serve,
    /// List current routes
    List,
    /// Add a route dynamically
    Add {
        /// Protocol (http, https, postgres, mysql, smtp)
        protocol: String,
        /// Routing key
        key: String,
        /// Backend address (host:port)
        backend: String,
        /// TLS mode for HTTPS routes (passthrough or terminate)
        #[arg(long)]
        tls_mode: Option<TlsMode>,
    },
    /// Remove a route
    Remove {
        /// Protocol (http, https, postgres, mysql, smtp)
        protocol: String,
        /// Routing key
        key: String,
    },
    /// Show daemon status
    Status,
    /// Reload configuration
    Reload,
    /// Run a command with automatic port allocation and route registration
    Run {
        /// Protocol (http, https, postgres, mysql, smtp)
        protocol: ProtocolKind,
        /// Routing key
        key: String,
        /// Detect port from stdout/stderr instead of allocating one
        #[arg(long)]
        detect_port: bool,
        /// Additional environment variable name to pass the port (e.g. DEV_API_PORT)
        #[arg(long)]
        port_env: Option<String>,
        /// TLS mode for HTTPS routes (passthrough or terminate)
        #[arg(long)]
        tls_mode: Option<TlsMode>,
        /// Command to run (use $PORT in args to substitute the allocated port)
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        None => run_server(cli.config, true).await,
        Some(Commands::Serve) => run_server(cli.config, false).await,
        Some(Commands::List) => {
            let resp = control::send_request(&control::Request::ListRoutes)
                .await
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            if let Some(routes) = resp.routes {
                if routes.is_empty() {
                    println!("No routes registered.");
                } else {
                    println!(
                        "{:<12} {:<20} {:<25} {:<8} {}",
                        "PROTOCOL", "KEY", "BACKEND", "SOURCE", "URL"
                    );
                    for r in &routes {
                        let url = match r.protocol {
                            ProtocolKind::Http => format!("http://{}.localhost:8080", r.key),
                            ProtocolKind::Https => format!("https://{}.localhost:8443", r.key),
                            _ => String::new(),
                        };
                        println!(
                            "{:<12} {:<20} {:<25} {:<8} {}",
                            r.protocol, r.key, r.backend, r.source, url
                        );
                    }
                }
            }
            Ok(())
        }
        Some(Commands::Add {
            protocol,
            key,
            backend,
            tls_mode,
        }) => {
            let protocol: ProtocolKind = protocol
                .parse()
                .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
            let resp = control::send_request(&control::Request::AddRoute {
                protocol,
                key: key.clone(),
                backend: backend.clone(),
                tls_mode,
            })
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            if resp.ok {
                println!("Route added: {}:{} -> {}", protocol, key, backend);
            } else {
                eprintln!(
                    "Error: {}",
                    resp.error.unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Remove { protocol, key }) => {
            let protocol: ProtocolKind = protocol
                .parse()
                .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;
            let resp = control::send_request(&control::Request::RemoveRoute {
                protocol,
                key: key.clone(),
            })
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            if resp.ok {
                println!("Route removed: {}:{}", protocol, key);
            } else {
                eprintln!(
                    "Error: {}",
                    resp.error.unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Status) => {
            match control::send_request(&control::Request::ListRoutes).await {
                Ok(resp) => {
                    if resp.ok {
                        let count = resp.routes.map(|r| r.len()).unwrap_or(0);
                        println!("Daemon is running. {} route(s) registered.", count);
                    } else {
                        println!("Daemon responded with error: {}", resp.error.unwrap_or_default());
                    }
                }
                Err(e) => {
                    eprintln!("Daemon is not running: {}", e);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        Some(Commands::Reload) => {
            eprintln!("Not yet implemented: requires control socket reload command");
            std::process::exit(1);
        }
        Some(Commands::Run {
            protocol,
            key,
            detect_port,
            port_env,
            tls_mode,
            command,
        }) => run::cmd_run(protocol, key, detect_port, port_env, tls_mode, command).await,
    }
}

async fn run_server(
    config_path: Option<PathBuf>,
    foreground: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = match &config_path {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };

    let is_root = nix::unistd::geteuid().is_root();

    // Initialize tracing
    init_tracing(&config);

    if !is_root {
        hosts::disable();
    }

    if foreground {
        print_banner(&config, is_root);
    } else {
        info!("name-route starting");
        if !is_root {
            warn!("Running without root privileges; DNS and /etc/hosts management disabled");
        }
    }

    // Config watch channel
    let (config_tx, config_rx) = watch::channel(config.clone());

    // Cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    let routing_table = router::new_shared_routing_table();

    // Load static routes from TOML [[routes]]
    {
        let mut table = routing_table.write().await;
        for route in &config.routes {
            let (host, port_str) = match route.backend.rsplit_once(':') {
                Some((h, p)) => (h, p),
                None => {
                    warn!(
                        protocol = %route.protocol,
                        key = %route.key,
                        backend = %route.backend,
                        "Invalid backend address (expected host:port), skipping"
                    );
                    continue;
                }
            };
            let port: u16 = match port_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    warn!(
                        protocol = %route.protocol,
                        key = %route.key,
                        backend = %route.backend,
                        "Invalid port in backend address, skipping"
                    );
                    continue;
                }
            };
            let addr: IpAddr = match host.parse() {
                Ok(a) => a,
                Err(_) => {
                    warn!(
                        protocol = %route.protocol,
                        key = %route.key,
                        backend = %route.backend,
                        "Invalid IP in backend address, skipping"
                    );
                    continue;
                }
            };

            let backend = router::Backend {
                source: "static".to_string(),
                container_name: route.key.clone(),
                addrs: vec![addr],
                port,
                tls_mode: route.tls_mode.unwrap_or(TlsMode::Passthrough),
            };

            let collision = table.insert(route.protocol, route.key.clone(), backend);
            if collision {
                warn!(
                    protocol = %route.protocol,
                    key = %route.key,
                    "Static route collision, overwriting previous entry"
                );
            }
            info!(
                protocol = %route.protocol,
                key = %route.key,
                backend = %route.backend,
                "Registered static route"
            );
        }
        info!(routes = table.len(), "Static routes loaded");
    }

    // Discovery integration (optional)
    if config.discovery.enabled && !config.discovery.paths.is_empty() {
        // Initial discovery scan
        let discovery_table = discovery::poll_once(&config);
        {
            let mut table = routing_table.write().await;
            for ((protocol, key), backend) in discovery_table.entries() {
                if table.lookup(*protocol, key).map_or(false, |b| b.source == "static") {
                    continue;
                }
                table.insert(*protocol, key.clone(), backend.clone());
            }
            info!(
                routes = table.len(),
                "Initial routing table built (static + discovery)"
            );
        }

        // Spawn discovery polling loop
        let disc_cancel = cancel.clone();
        let disc_table = routing_table.clone();
        let disc_config_rx = config_rx.clone();
        tokio::spawn(async move {
            discovery::polling_loop(disc_table, disc_config_rx, disc_cancel).await;
        });
    } else if config.discovery.enabled {
        info!("Discovery enabled but no paths configured");
    }

    // Docker integration (optional)
    if config.docker.enabled {
        match docker::connect_docker(&config).await {
            Ok(docker) => {
                // Initial Docker poll
                match docker::poll_once(&docker).await {
                    Ok(docker_table) => {
                        let mut table = routing_table.write().await;
                        // Merge Docker routes, preserving static and discovery routes
                        for ((protocol, key), backend) in docker_table.entries() {
                            if let Some(existing) = table.lookup(*protocol, key) {
                                if existing.source == "static"
                                    || existing.source == "discovery"
                                {
                                    continue;
                                }
                            }
                            table.insert(*protocol, key.clone(), backend.clone());
                        }
                        info!(
                            routes = table.len(),
                            "Initial routing table built (static + discovery + docker)"
                        );
                    }
                    Err(e) => {
                        error!(error = %e, "Initial Docker poll failed");
                    }
                }

                // Spawn Docker polling loop
                let docker_cancel = cancel.clone();
                let docker_table = routing_table.clone();
                let docker_config_rx = config_rx.clone();
                tokio::spawn(async move {
                    docker::polling_loop(docker, docker_table, docker_config_rx, docker_cancel)
                        .await;
                });
            }
            Err(e) => {
                warn!(error = %e, "Failed to connect to Docker, continuing without Docker");
            }
        }
    } else {
        info!("Docker integration disabled");
    }

    // Update /etc/hosts with HTTP route entries (root only)
    if is_root {
        let table = routing_table.read().await;
        hosts::sync(&table, &config.http.base_domain);
    }

    // Spawn signal handler
    let signal_cancel = cancel.clone();
    let signal_config_path = config_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    tokio::spawn(async move {
        signal::signal_handler(signal_config_path, config_tx, signal_cancel).await;
    });

    // Spawn control socket server
    {
        let ctrl_table = routing_table.clone();
        let ctrl_base_domain = config.http.base_domain.clone();
        let ctrl_cancel = cancel.clone();
        tokio::spawn(async move {
            control::run_control_server(ctrl_table, ctrl_base_domain, ctrl_cancel).await;
        });
    }

    // Spawn DNS server if enabled (root only for port 53)
    if config.dns.enabled && is_root {
        let dns_bind = config.dns.bind.clone();
        let dns_base_domain = config.http.base_domain.clone();
        let dns_cancel = cancel.clone();
        tokio::spawn(async move {
            if let Err(e) = dns::run_dns_server(&dns_bind, &dns_base_domain, dns_cancel).await {
                error!(error = %e, "DNS server failed");
            }
        });
    } else if !config.dns.enabled {
        info!("DNS server disabled");
    }

    // Spawn listeners for each configured protocol
    let mut listener_handles = Vec::new();

    for (name, listener_config) in &config.listeners {
        if !listener_config.enabled {
            info!(listener = %name, "Listener disabled");
            continue;
        }

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
            ProtocolKind::Http => {
                let handler = Arc::new(protocol::http::HttpHandler::new(
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
            ProtocolKind::Https => {
                // Create TLS acceptor only if terminate mode might be needed
                let needs_terminate = listener_config.tls_mode == Some(TlsMode::Terminate)
                    || config.routes.iter().any(|r| {
                        r.protocol == ProtocolKind::Https
                            && r.tls_mode == Some(TlsMode::Terminate)
                    });

                let tls_acceptor = if needs_terminate {
                    match tls::create_tls_acceptor(&config.tls) {
                        Ok(a) => Some(a),
                        Err(e) => {
                            error!(listener = %name, error = %e, "Failed to create TLS acceptor");
                            continue;
                        }
                    }
                } else {
                    None
                };

                let handler = Arc::new(protocol::https::HttpsHandler::new(
                    routing_table.clone(),
                    config_rx.clone(),
                    tls_acceptor,
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

    // Clean /etc/hosts entries (root only)
    if is_root {
        hosts::clean();
    }

    // Wait for listeners to finish
    for handle in listener_handles {
        let _ = handle.await;
    }

    info!("name-route stopped");
    Ok(())
}

fn print_banner(config: &Config, is_root: bool) {
    eprintln!("{}", BANNER);
    eprintln!("  version  {}", VERSION);
    eprintln!();

    // Sort listeners by name for stable output
    let mut listeners: Vec<_> = config.listeners.iter().collect();
    listeners.sort_by_key(|(name, _)| (*name).clone());

    for (_, lc) in &listeners {
        if !lc.enabled {
            continue;
        }
        eprintln!("  {:<12} {}", lc.protocol, lc.bind);
    }
    eprintln!();

    if !is_root {
        eprintln!("  WARNING: Running without root privileges.");
        eprintln!("  DNS server and /etc/hosts management require root.");
        eprintln!("  Restart with: sudo nameroute");
        eprintln!();
    }
}

fn init_tracing(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.general.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}
