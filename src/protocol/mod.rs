pub mod mysql;
pub mod postgres;
pub mod smtp;

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;

use serde::Deserialize;
use tokio::net::TcpStream;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolKind {
    Postgres,
    Mysql,
    Smtp,
}

impl fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolKind::Postgres => write!(f, "postgres"),
            ProtocolKind::Mysql => write!(f, "mysql"),
            ProtocolKind::Smtp => write!(f, "smtp"),
        }
    }
}

impl ProtocolKind {
    pub fn default_port(&self) -> u16 {
        match self {
            ProtocolKind::Postgres => 5432,
            ProtocolKind::Mysql => 3306,
            ProtocolKind::Smtp => 25,
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
