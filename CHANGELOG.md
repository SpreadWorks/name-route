# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

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
