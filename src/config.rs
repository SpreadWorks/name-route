use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::protocol::ProtocolKind;

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
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_log_output")]
    pub log_output: String,
    #[serde(default)]
    pub log_dir: Option<String>,
    pub run_as_user: Option<String>,
    pub run_as_group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerConfig {
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    #[serde(default = "default_mailbox_dir")]
    pub mailbox_dir: String,
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
}

fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_output() -> String {
    "stdout".to_string()
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

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            socket: default_docker_socket(),
            startup_retries: default_startup_retries(),
            startup_retry_interval: default_startup_retry_interval(),
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

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            mailbox_dir: default_mailbox_dir(),
            max_message_size: default_max_message_size(),
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
        if self.general.log_output == "file" && self.general.log_dir.is_none() {
            return Err(Error::Config(
                "log_dir must be set when log_output is 'file'".to_string(),
            ));
        }
        for (name, listener) in &self.listeners {
            if listener.bind.is_empty() {
                return Err(Error::Config(format!(
                    "listener '{}' has empty bind address",
                    name
                )));
            }
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
        assert_eq!(config.listeners.len(), 3);
        assert!(config.listeners.contains_key("postgres"));
        assert!(config.listeners.contains_key("mysql"));
        assert!(config.listeners.contains_key("smtp"));
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

    #[test]
    fn test_validate_log_dir_required() {
        let content = r#"
[general]
log_level = "info"
log_output = "file"
"#;
        let config: Config = toml::from_str(content).unwrap();
        assert!(config.validate().is_err());
    }
}
