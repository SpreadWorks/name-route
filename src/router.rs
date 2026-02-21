use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::protocol::ProtocolKind;

#[derive(Debug, Clone)]
pub struct Backend {
    pub container_id: String,
    pub container_name: String,
    pub addrs: Vec<IpAddr>,
    pub port: u16,
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
        let had_existing = self
            .routes
            .insert((protocol, normalized_key), backend)
            .is_some();
        had_existing
    }

    pub fn lookup(&self, protocol: ProtocolKind, key: &str) -> Option<&Backend> {
        let normalized_key = key.to_lowercase();
        self.routes.get(&(protocol, normalized_key))
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&(ProtocolKind, String), &Backend)> {
        self.routes.iter()
    }
}

pub type SharedRoutingTable = Arc<RwLock<RoutingTable>>;

pub fn new_shared_routing_table() -> SharedRoutingTable {
    Arc::new(RwLock::new(RoutingTable::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn make_backend(name: &str) -> Backend {
        Backend {
            container_id: format!("{}_id", name),
            container_name: name.to_string(),
            addrs: vec![IpAddr::V4(Ipv4Addr::new(172, 17, 0, 2))],
            port: 5432,
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
                container_id: "mysql_id".to_string(),
                container_name: "mysql".to_string(),
                addrs: vec![IpAddr::V4(Ipv4Addr::new(172, 17, 0, 3))],
                port: 3306,
            },
        );

        assert_eq!(table.len(), 2);
        let pg = table.lookup(ProtocolKind::Postgres, "app").unwrap();
        assert_eq!(pg.port, 5432);
        let my = table.lookup(ProtocolKind::Mysql, "app").unwrap();
        assert_eq!(my.port, 3306);
    }
}
