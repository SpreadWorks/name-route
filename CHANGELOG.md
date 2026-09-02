# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.5.0] - 2026-09-02

### Added

- Added repeatable `nameroute run --alias` routes so one child process and dynamically allocated port can serve multiple route keys atomically.
- Added Linux and macOS CI coverage for the Rust test suite.

### Changed

- `nameroute run` now manages the child process group, forwards SIGINT/SIGTERM, escalates unresponsive shutdowns to SIGKILL, and bounds control/output cleanup waits.
- Run routes now use invocation-specific ownership so stale cleanup cannot delete replacement routes.
- Docker and project discovery polling now preserve routes owned by active run sessions and manual additions.

### Fixed

- Routes are cleaned up when the child executable is missing, is not executable, or exits with an error.
- Direct SIGTERM no longer leaves run routes or managed child processes behind.
- Atomic registration and daemon-lifetime owner tombstones prevent partial aliases and delayed registration from resurrecting cleaned routes.
- Detect-port mode now serializes stdout/stderr candidates and safely handles short-lived children and inherited output pipes.
- Interactive terminal ownership is restored before cleanup, while background run jobs no longer seize the shell terminal.

## [0.4.0] - 2026-05-28

### Added

- Added `[http].base_domains` for accepting multiple HTTP Host / HTTPS SNI parent domains. The first domain is used for displayed URLs.
- Added route-level `base_domains` for static routes and project discovery routes.
- Added shorthand `.nameroute.toml` project configuration with top-level defaults and `[http]`, `[https]`, `[postgres]`, `[mysql]`, and `[smtp]` sections.
- Added separate draft and publish release scripts so draft assets can be tested before publishing.

### Changed

- `base_domain` is now deprecated in favor of `base_domains`, but remains supported for backward compatibility.
- HTTP/HTTPS routing, `/etc/hosts` management, and TLS domain file generation now respect global and route-level base domains.
- Explicit `[[routes]]` entries in `.nameroute.toml` now act as an override when shorthand sections generate the same `protocol/key`.
- `scripts/release.sh` is now a deprecated guard that points to `draft_release.sh` and `publish_release.sh`.

## [0.3.0] - 2026-03-26

### Changed

- SMTP mailbox layout changed from domain-only to recipient-specific paths: `/var/lib/name-route/mailbox/<to-domain>/<to-local>/`.
- SMTP saved filenames now include timestamp, sanitized envelope sender, and short id: `YYYYMMDD_HHMMSS_<from-mailaddress>_<shortid>.eml`.

### Added

- SMTP now generates a `.txt` preview file alongside each raw `.eml` with fields: `content-type`, `from`, `to`, `cc`, `subject`, `body`, and attachment filename list.
- SMTP preview for multipart messages now selects text body with priority `text/plain` then `text/html`.

### Fixed

- Raw SMTP `.eml` output is no longer modified; `From` header is not auto-injected when absent.

## [0.2.0] - 2026-03-01

### Changed

- Default listener bind address changed from `127.0.0.1` to `0.0.0.0` for all protocols (HTTP, HTTPS, PostgreSQL, MySQL, SMTP). This allows Docker containers to connect to nameroute via `host.docker.internal` without extra configuration. The management API remains bound to `127.0.0.1`.

## [0.1.0] - 2026-02-26

Initial release. **Name it. Route it.**

name-route is a local TCP L7 router for development environments. Instead of managing port numbers across multiple projects, access your services by **name** — a subdomain, a database name, or a mail domain.

### Added

- **5 protocol support** — HTTP, HTTPS, PostgreSQL, MySQL, and SMTP routing at the application layer
- **`nameroute run`** — Wraps any command, auto-allocating a port and registering the route
- **Docker auto-discovery** — Detects routes from container labels in real time
- **Project discovery** — Scans for `.nameroute.toml` files in project directories
- HTTP routing by subdomain (`http://myapp.localhost:8080`)
- Multi-level subdomain support (e.g. `api.myapp.localhost`)
- Static routes via TOML config
- HTTPS passthrough mode (forwards TLS as-is to backend)
- HTTPS terminate mode (terminates TLS locally, forwards plain HTTP)
- Dynamic TLS domain management via `nameroute tls-domain` commands
- WebSocket transparent relay
- `/etc/hosts` auto-management for HTTP routes
- Management API on `127.0.0.1` for route control
- Backend health checking with configurable intervals
- Graceful shutdown on SIGTERM/SIGINT
- Privilege dropping after binding to privileged ports
- Pre-built binaries: Linux (x86_64, aarch64), macOS (x86_64, Apple Silicon)
- deb / rpm packages
- Homebrew: `brew install SpreadWorks/tap/nameroute`

[0.2.0]: https://github.com/SpreadWorks/name-route/releases/tag/v0.2.0
[0.1.0]: https://github.com/SpreadWorks/name-route/releases/tag/v0.1.0
[0.3.0]: https://github.com/SpreadWorks/name-route/releases/tag/v0.3.0
[0.4.0]: https://github.com/SpreadWorks/name-route/releases/tag/v0.4.0
[0.5.0]: https://github.com/SpreadWorks/name-route/releases/tag/v0.5.0
