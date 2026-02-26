use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::protocol::{ProtocolKind, TlsMode};

#[derive(Debug, Clone)]
pub struct Backend {
    pub source: String,
    pub container_name: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
    pub tls_mode: TlsMode,
}

#[derive(Debug, Clone, Default)]
pub struct RoutingTable {
    routes: HashMap<(ProtocolKind, String), Backend>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Insert a route. Key is lowercased. Returns true if there was a collision.
    pub fn insert(&mut self, protocol: ProtocolKind, key: String, backend: Backend) -> bool {
        let normalized_key = key.to_lowercase();
        self.routes
            .insert((protocol, normalized_key), backend)
            .is_some()
    }

    pub fn lookup(&self, protocol: ProtocolKind, key: &str) -> Option<&Backend> {
        let normalized_key = key.to_lowercase();
        self.routes.get(&(protocol, normalized_key))
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Remove all routes with the given source.
    pub fn remove_by_source(&mut self, source: &str) {
        self.routes.retain(|_, b| b.source != source);
    }

    /// Remove a specific route by protocol and key. Returns true if it existed.
    pub fn remove(&mut self, protocol: ProtocolKind, key: &str) -> bool {
        let normalized_key = key.to_lowercase();
        self.routes.remove(&(protocol, normalized_key)).is_some()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&(ProtocolKind, String), &Backend)> {
        self.routes.iter()
    }
}

pub type SharedRoutingTable = Arc<RwLock<RoutingTable>>;

pub fn new_shared_routing_table() -> SharedRoutingTable {
    Arc::new(RwLock::new(RoutingTable::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
}

pub type SharedHealthMap = Arc<RwLock<HashMap<(ProtocolKind, String), HealthStatus>>>;

pub fn new_shared_health_map() -> SharedHealthMap {
    Arc::new(RwLock::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn make_backend(name: &str) -> Backend {
        Backend {
            source: "docker".to_string(),
            container_name: name.to_string(),
            addrs: vec![IpAddr::V4(Ipv4Addr::new(172, 17, 0, 2))],
            port: 5432,
            tls_mode: TlsMode::Passthrough,
        }
    }

    #[test]
    fn test_insert_and_lookup() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Postgres,
            "mydb".to_string(),
            make_backend("pg1"),
        );

        assert!(table
            .lookup(ProtocolKind::Postgres, "mydb")
            .is_some());
        assert!(table
            .lookup(ProtocolKind::Postgres, "MYDB")
            .is_some());
        assert!(table
            .lookup(ProtocolKind::Postgres, "other")
            .is_none());
        assert!(table
            .lookup(ProtocolKind::Mysql, "mydb")
            .is_none());
    }

    #[test]
    fn test_collision_returns_true() {
        let mut table = RoutingTable::new();
        let first = table.insert(
            ProtocolKind::Postgres,
            "mydb".to_string(),
            make_backend("pg1"),
        );
        assert!(!first);

        let second = table.insert(
            ProtocolKind::Postgres,
            "mydb".to_string(),
            make_backend("pg2"),
        );
        assert!(second);
    }

    #[test]
    fn test_case_insensitive() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Postgres,
            "MyDB".to_string(),
            make_backend("pg1"),
        );

        assert!(table
            .lookup(ProtocolKind::Postgres, "mydb")
            .is_some());
        assert!(table
            .lookup(ProtocolKind::Postgres, "MYDB")
            .is_some());
        assert!(table
            .lookup(ProtocolKind::Postgres, "MyDB")
            .is_some());
    }

    #[test]
    fn test_different_protocols_same_key() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Postgres,
            "app".to_string(),
            make_backend("pg"),
        );
        table.insert(
            ProtocolKind::Mysql,
            "app".to_string(),
            Backend {
                source: "docker".to_string(),
                container_name: "mysql".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(172, 17, 0, 3))],
                port: 3306,
                tls_mode: TlsMode::Passthrough,
            },
        );

        assert_eq!(table.len(), 2);
        let pg = table.lookup(ProtocolKind::Postgres, "app").unwrap();
        assert_eq!(pg.port, 5432);
        let my = table.lookup(ProtocolKind::Mysql, "app").unwrap();
        assert_eq!(my.port, 3306);
    }

    // ---- Scenario tests: HTTPS route management ----

    /// HTTPS terminate route should preserve tls_mode.
    #[test]
    fn test_https_terminate_tls_mode() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Https,
            "myapp".to_string(),
            Backend {
                source: "run".to_string(),
                container_name: "myapp".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
                port: 3000,
                tls_mode: TlsMode::Terminate,
            },
        );

        let b = table.lookup(ProtocolKind::Https, "myapp").unwrap();
        assert_eq!(b.tls_mode, TlsMode::Terminate);
        assert_eq!(b.port, 3000);
    }

    /// Route removal should succeed and subsequent lookup should return None.
    #[test]
    fn test_remove_route() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Https,
            "myapp".to_string(),
            make_backend("app"),
        );
        assert!(table.lookup(ProtocolKind::Https, "myapp").is_some());

        let removed = table.remove(ProtocolKind::Https, "myapp");
        assert!(removed);
        assert!(table.lookup(ProtocolKind::Https, "myapp").is_none());

        // Removing again should return false
        assert!(!table.remove(ProtocolKind::Https, "myapp"));
    }

    /// remove_by_source should only remove routes with matching source.
    /// Simulates Docker polling removing old docker routes while keeping static/run routes.
    #[test]
    fn test_remove_by_source_preserves_others() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Http,
            "static-app".to_string(),
            Backend {
                source: "static".to_string(),
                container_name: "static-app".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
                port: 3000,
                tls_mode: TlsMode::Passthrough,
            },
        );
        table.insert(
            ProtocolKind::Http,
            "docker-app".to_string(),
            Backend {
                source: "docker".to_string(),
                container_name: "docker-app".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(172, 17, 0, 2))],
                port: 80,
                tls_mode: TlsMode::Passthrough,
            },
        );
        table.insert(
            ProtocolKind::Https,
            "run-app".to_string(),
            Backend {
                source: "run".to_string(),
                container_name: "run-app".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
                port: 4000,
                tls_mode: TlsMode::Terminate,
            },
        );

        assert_eq!(table.len(), 3);
        table.remove_by_source("docker");
        assert_eq!(table.len(), 2);
        assert!(table.lookup(ProtocolKind::Http, "static-app").is_some());
        assert!(table.lookup(ProtocolKind::Http, "docker-app").is_none());
        assert!(table.lookup(ProtocolKind::Https, "run-app").is_some());
    }

    /// Multi-level key routing: "image.echub" should be stored and found correctly.
    #[test]
    fn test_multilevel_key_routing() {
        let mut table = RoutingTable::new();
        table.insert(
            ProtocolKind::Https,
            "image.echub".to_string(),
            Backend {
                source: "run".to_string(),
                container_name: "image.echub".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
                port: 5000,
                tls_mode: TlsMode::Terminate,
            },
        );

        // Exact key lookup
        assert!(table.lookup(ProtocolKind::Https, "image.echub").is_some());
        // Case insensitive
        assert!(table.lookup(ProtocolKind::Https, "IMAGE.ECHUB").is_some());
        // Partial key should not match
        assert!(table.lookup(ProtocolKind::Https, "image").is_none());
        assert!(table.lookup(ProtocolKind::Https, "echub").is_none());
    }
}
