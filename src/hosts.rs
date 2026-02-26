use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tracing::{debug, error, info};

use crate::protocol::ProtocolKind;
use crate::router::RoutingTable;

const HOSTS_PATH: &str = "/etc/hosts";
const BEGIN_MARKER: &str = "# --- BEGIN name-route ---";
const END_MARKER: &str = "# --- END name-route ---";

/// Set to true when not root or after the first write failure; all subsequent calls become no-ops.
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Serialize concurrent writes to /etc/hosts from multiple tasks
/// (Docker polling, discovery polling, control server).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Call at startup to disable hosts management when not running as root.
pub fn disable() {
    DISABLED.store(true, Ordering::Relaxed);
}

/// Update /etc/hosts with entries for all HTTP routes in the table.
pub fn sync(table: &RoutingTable, base_domain: &str) {
    if DISABLED.load(Ordering::Relaxed) {
        return;
    }

    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut hostnames: Vec<String> = table
        .entries()
        .filter(|((protocol, _), _)| {
            *protocol == ProtocolKind::Http || *protocol == ProtocolKind::Https
        })
        .map(|((_, key), _)| format!("{}.{}", key, base_domain))
        .collect();
    hostnames.sort();
    hostnames.dedup();

    match write_hosts(HOSTS_PATH, &hostnames) {
        Ok(()) => {
            if hostnames.is_empty() {
                debug!("Cleared /etc/hosts entries");
            } else {
                info!(count = hostnames.len(), "Updated /etc/hosts");
            }
        }
        Err(e) => {
            error!(error = %e, "Failed to update /etc/hosts, disabling hosts management");
            DISABLED.store(true, Ordering::Relaxed);
        }
    }
}

/// Remove all name-route managed entries from /etc/hosts.
pub fn clean() {
    if DISABLED.load(Ordering::Relaxed) {
        return;
    }

    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    match write_hosts(HOSTS_PATH, &[]) {
        Ok(()) => {
            info!("Cleaned /etc/hosts entries");
        }
        Err(e) => {
            error!(error = %e, "Failed to clean /etc/hosts");
        }
    }
}

fn write_hosts(path: &str, hostnames: &[String]) -> std::io::Result<()> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let new_content = rebuild_hosts(&content, hostnames);

    // Atomic write: write to a temp file in the same directory, then rename.
    // This prevents /etc/hosts corruption if the process crashes mid-write.
    let dir = std::path::Path::new(path)
        .parent()
        .unwrap_or(std::path::Path::new("/tmp"));
    let tmp_path = dir.join(".hosts.nameroute.tmp");
    fs::write(&tmp_path, &new_content)?;
    fs::rename(&tmp_path, path)?;

    Ok(())
}

/// Rebuild hosts file content: remove old managed block, optionally insert new one.
fn rebuild_hosts(content: &str, hostnames: &[String]) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut inside_block = false;

    for line in content.lines() {
        if line.trim() == BEGIN_MARKER {
            inside_block = true;
            continue;
        }
        if line.trim() == END_MARKER {
            inside_block = false;
            continue;
        }
        if !inside_block {
            lines.push(line);
        }
    }

    // Remove trailing empty lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    let mut output = String::new();
    for line in &lines {
        output.push_str(line);
        output.push('\n');
    }

    if !hostnames.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(BEGIN_MARKER);
        output.push('\n');
        for hostname in hostnames {
            output.push_str(&format!("127.0.0.1 {}\n", hostname));
            output.push_str(&format!("::1 {}\n", hostname));
        }
        output.push_str(END_MARKER);
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rebuild_empty_hosts_with_entries() {
        let result = rebuild_hosts(
            "",
            &["api1.localhost".to_string(), "dev1.localhost".to_string()],
        );
        let expected = format!(
            "{}\n127.0.0.1 api1.localhost\n::1 api1.localhost\n127.0.0.1 dev1.localhost\n::1 dev1.localhost\n{}\n",
            BEGIN_MARKER, END_MARKER
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_rebuild_preserves_existing() {
        let existing = "127.0.0.1 localhost\n::1 localhost\n";
        let result = rebuild_hosts(existing, &["dev1.localhost".to_string()]);
        assert!(result.starts_with("127.0.0.1 localhost\n::1 localhost\n"));
        assert!(result.contains(BEGIN_MARKER));
        assert!(result.contains("127.0.0.1 dev1.localhost"));
        assert!(result.contains("::1 dev1.localhost"));
        assert!(result.contains(END_MARKER));
    }

    #[test]
    fn test_rebuild_replaces_old_block() {
        let existing = format!(
            "127.0.0.1 localhost\n{}\n127.0.0.1 old.localhost\n::1 old.localhost\n{}\n",
            BEGIN_MARKER, END_MARKER
        );
        let result = rebuild_hosts(&existing, &["new.localhost".to_string()]);
        assert!(!result.contains("old.localhost"));
        assert!(result.contains("new.localhost"));
        assert_eq!(result.matches(BEGIN_MARKER).count(), 1);
        assert_eq!(result.matches(END_MARKER).count(), 1);
    }

    #[test]
    fn test_rebuild_clean_removes_block() {
        let existing = format!(
            "127.0.0.1 localhost\n{}\n127.0.0.1 dev1.localhost\n::1 dev1.localhost\n{}\n",
            BEGIN_MARKER, END_MARKER
        );
        let result = rebuild_hosts(&existing, &[]);
        assert!(!result.contains(BEGIN_MARKER));
        assert!(!result.contains(END_MARKER));
        assert!(!result.contains("dev1.localhost"));
        assert!(result.contains("127.0.0.1 localhost"));
    }

    #[test]
    fn test_rebuild_no_entries_no_block() {
        let result = rebuild_hosts("127.0.0.1 localhost\n", &[]);
        assert!(!result.contains(BEGIN_MARKER));
        assert_eq!(result, "127.0.0.1 localhost\n");
    }

    #[test]
    fn test_rebuild_handles_no_trailing_newline() {
        let existing = "127.0.0.1 localhost";
        let result = rebuild_hosts(existing, &["dev1.localhost".to_string()]);
        assert!(result.starts_with("127.0.0.1 localhost\n"));
        assert!(result.contains(BEGIN_MARKER));
    }

    #[test]
    fn test_write_hosts_creates_file() {
        let dir = std::env::temp_dir().join("name-route-test-hosts");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hosts");
        let path_str = path.to_str().unwrap();

        // Write entries
        write_hosts(path_str, &["dev1.localhost".to_string()]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("127.0.0.1 dev1.localhost"));
        assert!(content.contains("::1 dev1.localhost"));
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains(END_MARKER));

        // Clean
        write_hosts(path_str, &[]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(BEGIN_MARKER));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_rebuild_preserves_comments_and_blanks() {
        let existing = "# system hosts\n127.0.0.1 localhost\n\n# custom\n192.168.1.1 myserver\n";
        let result = rebuild_hosts(existing, &["dev1.localhost".to_string()]);
        assert!(result.contains("# system hosts"));
        assert!(result.contains("# custom"));
        assert!(result.contains("192.168.1.1 myserver"));
    }
}
