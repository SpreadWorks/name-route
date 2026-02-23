pub mod http;
pub mod https;
pub mod mysql;
pub mod postgres;
pub mod smtp;

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKind {
    Postgres,
    Mysql,
    Smtp,
    Http,
    Https,
}

impl fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolKind::Postgres => write!(f, "postgres"),
            ProtocolKind::Mysql => write!(f, "mysql"),
            ProtocolKind::Smtp => write!(f, "smtp"),
            ProtocolKind::Http => write!(f, "http"),
            ProtocolKind::Https => write!(f, "https"),
        }
    }
}

impl FromStr for ProtocolKind {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            "smtp" => Ok(Self::Smtp),
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            other => Err(format!("unknown protocol: {}", other)),
        }
    }
}

impl ProtocolKind {
    pub fn default_port(&self) -> u16 {
        match self {
            ProtocolKind::Postgres => 5432,
            ProtocolKind::Mysql => 3306,
            ProtocolKind::Smtp => 25,
            ProtocolKind::Http => 80,
            ProtocolKind::Https => 443,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    Passthrough,
    Terminate,
}

impl Default for TlsMode {
    fn default() -> Self {
        Self::Passthrough
    }
}

impl fmt::Display for TlsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TlsMode::Passthrough => write!(f, "passthrough"),
            TlsMode::Terminate => write!(f, "terminate"),
        }
    }
}

impl FromStr for TlsMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "passthrough" => Ok(Self::Passthrough),
            "terminate" => Ok(Self::Terminate),
            other => Err(format!("unknown tls_mode: {}", other)),
        }
    }
}

pub trait ProtocolHandler: Send + Sync + 'static {
    fn handle_connection(
        &self,
        client: TcpStream,
        peer: SocketAddr,
    ) -> impl Future<Output = Result<()>> + Send;
}
