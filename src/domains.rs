use std::io;
use std::path::Path;

use tracing::warn;

const DEFAULT_DOMAINS_PATH: &str = "/etc/nameroute/domains";

/// Return the path to the domains file.
pub fn domains_path() -> &'static str {
    DEFAULT_DOMAINS_PATH
}

/// Compute the wildcard domain pattern needed for a routing key.
///
/// - `"myapp"` → `"*.localhost"`
/// - `"frontend.echub"` → `"*.echub.localhost"`
/// - `"api.frontend.echub"` → `"*.frontend.echub.localhost"`
pub fn wildcard_for_key(key: &str, base_domain: &str) -> String {
    if let Some((_first, rest)) = key.split_once('.') {
        format!("*.{}.{}", rest, base_domain)
    } else {
        format!("*.{}", base_domain)
    }
}

/// Ensure that `pattern` exists in the domains file at `path`.
/// Appends it if missing. Returns `true` if a new line was added.
pub fn ensure_domain(path: &Path, pattern: &str) -> io::Result<bool> {
    // Read existing content (file may not exist yet)
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Create parent directory if needed
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            String::new()
        }
        Err(e) => return Err(e),
    };

    // Check if pattern already present (one pattern per line)
    for line in content.lines() {
        if line.trim() == pattern {
            return Ok(false);
        }
    }

    // Append the pattern
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", pattern)?;

    Ok(true)
}

/// Ensure the wildcard pattern for a routing key exists in the domains file.
/// Also ensures the base wildcard pattern (e.g. `*.localhost`) is always present.
/// Logs a warning with regeneration instructions when a new pattern is added.
pub fn ensure_domain_for_key(key: &str, base_domain: &str, tls_cert: &str, tls_key: &str) {
    let path = Path::new(domains_path());

    // Always ensure the base wildcard is present
    let base_pattern = format!("*.{}", base_domain);
    let base_added = match ensure_domain(path, &base_pattern) {
        Ok(added) => added,
        Err(e) => {
            tracing::debug!(
                error = %e,
                path = %path.display(),
                "Failed to update domains file (this is expected without root)"
            );
            return;
        }
    };

    // Ensure the key-specific wildcard pattern
    let pattern = wildcard_for_key(key, base_domain);
    let key_added = if pattern != base_pattern {
        match ensure_domain(path, &pattern) {
            Ok(added) => added,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    path = %path.display(),
                    "Failed to update domains file (this is expected without root)"
                );
                return;
            }
        }
    } else {
        false
    };

    if base_added || key_added {
        let added = if key_added { &pattern } else { &base_pattern };
        warn!(
            pattern = %added,
            "New domain pattern added to {}. Regenerate certificate:\n  \
             sudo xargs mkcert -key-file {} -cert-file {} < {}\n  \
             sudo systemctl restart nameroute",
            path.display(),
            tls_key,
            tls_cert,
            path.display(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_for_key_simple() {
        assert_eq!(wildcard_for_key("myapp", "localhost"), "*.localhost");
    }

    #[test]
    fn test_wildcard_for_key_two_level() {
        assert_eq!(
            wildcard_for_key("frontend.echub", "localhost"),
            "*.echub.localhost"
        );
    }

    #[test]
    fn test_wildcard_for_key_three_level() {
        assert_eq!(
            wildcard_for_key("api.frontend.echub", "localhost"),
            "*.frontend.echub.localhost"
        );
    }

    #[test]
    fn test_ensure_domain_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        assert!(ensure_domain(&path, "*.localhost").unwrap());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim(), "*.localhost");
    }

    #[test]
    fn test_ensure_domain_no_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        assert!(ensure_domain(&path, "*.localhost").unwrap());
        assert!(!ensure_domain(&path, "*.localhost").unwrap());

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("*.localhost").count(), 1);
    }

    #[test]
    fn test_ensure_domain_multiple_patterns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        assert!(ensure_domain(&path, "*.localhost").unwrap());
        assert!(ensure_domain(&path, "*.echub.localhost").unwrap());

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "*.localhost");
        assert_eq!(lines[1], "*.echub.localhost");
    }

    // ---- Scenario tests simulating real-world usage ----

    /// Simulate: `nameroute run https myapp --tls-mode terminate`
    /// A simple single-level key should produce *.localhost in the domains file.
    #[test]
    fn test_scenario_simple_route_includes_base_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        let key = "myapp";
        let base = "localhost";
        let base_pattern = format!("*.{}", base);
        let pattern = wildcard_for_key(key, base);

        // The key-specific pattern should equal the base pattern
        assert_eq!(pattern, base_pattern);

        ensure_domain(&path, &base_pattern).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec!["*.localhost"]);
    }

    /// Simulate: `nameroute run https image.echub --tls-mode terminate`
    /// Must produce BOTH *.localhost AND *.echub.localhost in the domains file.
    /// This caught a real bug where *.localhost was missing.
    #[test]
    fn test_scenario_multilevel_route_includes_base_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        let key = "image.echub";
        let base = "localhost";
        let base_pattern = format!("*.{}", base);
        let pattern = wildcard_for_key(key, base);

        // Base and key-specific patterns must differ
        assert_ne!(pattern, base_pattern);
        assert_eq!(pattern, "*.echub.localhost");

        // Simulate what ensure_domain_for_key does
        ensure_domain(&path, &base_pattern).unwrap();
        ensure_domain(&path, &pattern).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec!["*.localhost", "*.echub.localhost"]);
    }

    /// Simulate registering multiple routes over time:
    ///   nameroute run https myapp --tls-mode terminate
    ///   nameroute run https image.echub --tls-mode terminate
    ///   nameroute run https api.frontend.echub --tls-mode terminate
    ///   nameroute run https dashboard --tls-mode terminate
    /// The domains file should contain all needed patterns without duplicates.
    #[test]
    fn test_scenario_multiple_routes_accumulate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        let keys = vec!["myapp", "image.echub", "api.frontend.echub", "dashboard"];
        let base = "localhost";

        for key in &keys {
            let base_pattern = format!("*.{}", base);
            let pattern = wildcard_for_key(key, base);
            ensure_domain(&path, &base_pattern).unwrap();
            if pattern != base_pattern {
                ensure_domain(&path, &pattern).unwrap();
            }
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(
            lines,
            vec![
                "*.localhost",
                "*.echub.localhost",
                "*.frontend.echub.localhost",
            ]
        );
    }

    /// The domains file content must be usable as `xargs mkcert < domains`.
    /// Each line should be a valid mkcert domain argument.
    #[test]
    fn test_scenario_domains_file_valid_for_mkcert() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        let keys = vec!["myapp", "image.echub", "api.frontend.echub"];
        let base = "localhost";

        for key in &keys {
            let base_pattern = format!("*.{}", base);
            let pattern = wildcard_for_key(key, base);
            ensure_domain(&path, &base_pattern).unwrap();
            if pattern != base_pattern {
                ensure_domain(&path, &pattern).unwrap();
            }
        }

        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            let line = line.trim();
            assert!(!line.is_empty(), "empty line in domains file");
            assert!(
                line.starts_with("*."),
                "pattern '{}' does not start with '*.'",
                line
            );
            assert!(!line.contains(' '), "pattern '{}' contains spaces", line);
        }
    }

    /// Custom base domain (e.g. mysite.local) should work the same way.
    #[test]
    fn test_scenario_custom_base_domain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("domains");

        let base = "mysite.local";
        let keys = vec!["app", "api.app"];

        for key in &keys {
            let base_pattern = format!("*.{}", base);
            let pattern = wildcard_for_key(key, base);
            ensure_domain(&path, &base_pattern).unwrap();
            if pattern != base_pattern {
                ensure_domain(&path, &pattern).unwrap();
            }
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec!["*.mysite.local", "*.app.mysite.local"]);
    }
}
