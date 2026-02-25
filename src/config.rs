use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::protocol::{ProtocolKind, TlsMode};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    #[serde(default)]
    pub docker: DockerConfig,
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default)]
    pub listeners: HashMap<String, ListenerConfig>,
    #[serde(default)]
    pub smtp: SmtpConfig,
    #[serde(default)]
    pub routes: Vec<StaticRoute>,
    #[serde(default)]
    pub http: HttpConfig,
    #[serde(default)]
    pub dns: DnsConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub health_check: HealthCheckConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    pub run_as_user: Option<String>,
    pub run_as_group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
    #[serde(default = "default_docker_socket")]
    pub socket: String,
    #[serde(default = "default_startup_retries")]
    pub startup_retries: u32,
    #[serde(default = "default_startup_retry_interval")]
    pub startup_retry_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackendConfig {
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: u64,
    #[serde(default = "default_connect_retries")]
    pub connect_retries: u32,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListenerConfig {
    pub protocol: ProtocolKind,
    pub bind: String,
    #[serde(default)]
    pub tls_mode: Option<TlsMode>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    #[serde(default = "default_mailbox_dir")]
    pub mailbox_dir: String,
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StaticRoute {
    pub protocol: ProtocolKind,
    pub key: String,
    pub backend: String,
    #[serde(default)]
    pub tls_mode: Option<TlsMode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpConfig {
    #[serde(default = "default_base_domain")]
    pub base_domain: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DnsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_dns_bind")]
    pub bind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    #[serde(default)]
    pub cert: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthCheckConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_health_check_interval")]
    pub interval: u64,
    #[serde(default = "default_health_check_timeout")]
    pub timeout: u64,
}

fn default_true() -> bool {
    true
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_poll_interval() -> u64 {
    3
}
fn default_docker_socket() -> String {
    "/var/run/docker.sock".to_string()
}
fn default_startup_retries() -> u32 {
    10
}
fn default_startup_retry_interval() -> u64 {
    1
}
fn default_connect_timeout() -> u64 {
    5
}
fn default_connect_retries() -> u32 {
    3
}
fn default_idle_timeout() -> u64 {
    10
}
fn default_mailbox_dir() -> String {
    "/var/lib/name-route/mailbox".to_string()
}
fn default_max_message_size() -> usize {
    10_485_760
}
fn default_base_domain() -> String {
    "localhost".to_string()
}
fn default_dns_bind() -> String {
    "127.0.0.1:53".to_string()
}
fn default_health_check_interval() -> u64 {
    5
}
fn default_health_check_timeout() -> u64 {
    2
}
impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            poll_interval: default_poll_interval(),
            socket: default_docker_socket(),
            startup_retries: default_startup_retries(),
            startup_retry_interval: default_startup_retry_interval(),
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            base_domain: default_base_domain(),
        }
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            bind: default_dns_bind(),
        }
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            connect_timeout: default_connect_timeout(),
            connect_retries: default_connect_retries(),
            idle_timeout: default_idle_timeout(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            run_as_user: None,
            run_as_group: None,
        }
    }
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            paths: Vec::new(),
            poll_interval: default_poll_interval(),
        }
    }
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            mailbox_dir: default_mailbox_dir(),
            max_message_size: default_max_message_size(),
        }
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            cert: None,
            key: None,
        }
    }
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            interval: default_health_check_interval(),
            timeout: default_health_check_timeout(),
        }
    }
}

fn default_listeners() -> HashMap<String, ListenerConfig> {
    let mut m = HashMap::new();
    m.insert("postgres".to_string(), ListenerConfig {
        protocol: ProtocolKind::Postgres,
        bind: "127.0.0.1:15432".to_string(),
        tls_mode: None,
        enabled: true,
    });
    m.insert("mysql".to_string(), ListenerConfig {
        protocol: ProtocolKind::Mysql,
        bind: "127.0.0.1:13306".to_string(),
        tls_mode: None,
        enabled: true,
    });
    m.insert("http".to_string(), ListenerConfig {
        protocol: ProtocolKind::Http,
        bind: "127.0.0.1:8080".to_string(),
        tls_mode: None,
        enabled: true,
    });
    m.insert("smtp".to_string(), ListenerConfig {
        protocol: ProtocolKind::Smtp,
        bind: "127.0.0.1:10025".to_string(),
        tls_mode: None,
        enabled: true,
    });
    m.insert("https".to_string(), ListenerConfig {
        protocol: ProtocolKind::Https,
        bind: "127.0.0.1:8443".to_string(),
        tls_mode: None,
        enabled: true,
    });
    m
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            docker: DockerConfig::default(),
            backend: BackendConfig::default(),
            listeners: default_listeners(),
            smtp: SmtpConfig::default(),
            routes: Vec::new(),
            http: HttpConfig::default(),
            dns: DnsConfig::default(),
            discovery: DiscoveryConfig::default(),
            tls: TlsConfig::default(),
            health_check: HealthCheckConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("Failed to read config file {:?}: {}", path, e)))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse config: {}", e)))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for (name, listener) in &self.listeners {
            if listener.bind.is_empty() {
                return Err(Error::Config(format!(
                    "listener '{}' has empty bind address",
                    name
                )));
            }
            if listener.tls_mode.is_some() && listener.protocol != ProtocolKind::Https {
                return Err(Error::Config(format!(
                    "listener '{}': tls_mode can only be set on protocol = \"https\"",
                    name
                )));
            }
        }
        for route in &self.routes {
            if route.tls_mode.is_some() && route.protocol != ProtocolKind::Https {
                return Err(Error::Config(format!(
                    "route '{}': tls_mode can only be set on protocol = \"https\"",
                    route.key
                )));
            }
        }
        let needs_terminate = self.listeners.values().any(|lc| {
            lc.protocol == ProtocolKind::Https
                && lc.enabled
                && lc.tls_mode == Some(TlsMode::Terminate)
        }) || self.routes.iter().any(|r| {
            r.protocol == ProtocolKind::Https && r.tls_mode == Some(TlsMode::Terminate)
        });
        if needs_terminate && (self.tls.cert.is_none() || self.tls.key.is_none()) {
            return Err(Error::Config(
                "TLS terminate mode requires [tls] cert and key. \
                 Generate certs with: mkcert -key-file key.pem -cert-file cert.pem \"*.localhost\""
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_example_config() {
        let content = std::fs::read_to_string("config.example.toml").unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.general.log_level, "info");
        assert_eq!(config.docker.poll_interval, 3);
        assert_eq!(config.backend.connect_timeout, 5);
        assert_eq!(config.listeners.len(), 5);
        assert!(config.listeners.contains_key("postgres"));
        assert!(config.listeners.contains_key("mysql"));
        assert!(config.listeners.contains_key("http"));
        assert!(config.listeners.contains_key("smtp"));
        assert!(config.listeners.contains_key("https"));
        assert_eq!(config.smtp.max_message_size, 10_485_760);
    }

    #[test]
    fn test_default_values() {
        let content = r#"
[general]
log_level = "debug"
"#;
        let config: Config = toml::from_str(content).unwrap();
        assert_eq!(config.docker.poll_interval, 3);
        assert_eq!(config.backend.connect_retries, 3);
        assert_eq!(config.smtp.max_message_size, 10_485_760);
    }

}
