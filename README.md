[日本語](docs/ja/README.md)

# name-route

## Name it. Route it.

As you juggle multiple projects — or run AI-driven parallel development — the number of ports you need to track keeps growing. name-route lets you access local services by **name** instead of port number. You only need to remember one port per protocol. After that, just use a hostname or database name and name-route handles the rest.

> **Note:** name-route is designed exclusively for local development. It is not intended for production use or public-facing deployments.


## How it works

<p align="center">
  <img src="docs/images/how-it-works.svg" alt="How it works" width="100%" />
</p>

Remember one port per protocol — name-route routes by name.

- **HTTP** — The subdomain is the routing key: `http://myapp.localhost:8080`
- **PostgreSQL / MySQL** — The database name is the routing key
- **SMTP** — The recipient domain is the routing key: `smtp://myapp.localhost:10025`

No matter what port backends actually listen on, developers only think about names.


## Features

- **5 protocols** — Routes HTTP, HTTPS, PostgreSQL, MySQL, and SMTP at the protocol level
- **HTTPS (passthrough / terminate)** — Passthrough by default (forwards TLS as-is, no certs needed). Terminate mode uses local certs (e.g. mkcert) to terminate TLS and forward plain HTTP to backends
- **WebSocket transparent** — HTTP proxy uses transparent relay, so WebSocket connections (Next.js HMR, etc.) just work
- **`nameroute run`** — Auto-allocates a free port, registers the route, and runs your command. No more port management
- **Docker auto-detection** — Discovers routes from container labels. Tracks container start/stop automatically
- **Discovery** — Auto-detects `.nameroute.toml` files in your projects. Git-friendly
- **Static routes** — Define routes in TOML config for non-Docker services
- **Multi-level subdomains** — Supports `api.myapp.localhost` style nested subdomains
- **Zero config startup** — All protocol listeners start by default. Config file is optional
- **Built-in DNS server** — Resolves `*.localhost` to `127.0.0.1` (when running as root; not needed for browsers)
- **/etc/hosts management** — Automatically adds/removes hostnames for HTTP routes (when running as root; not needed for browsers)
- **Single binary** — No dependencies. Drop one file and go


## Install

```bash
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-gnu -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/
```

<details>
<summary>Homebrew (macOS / Linux)</summary>

```bash
brew install SpreadWorks/tap/nameroute
```

</details>

<details>
<summary>Debian / Ubuntu</summary>

#### x86_64

```bash
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute_amd64.deb
sudo dpkg -i nameroute_amd64.deb
```

#### ARM64

```bash
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute_arm64.deb
sudo dpkg -i nameroute_arm64.deb
```

</details>

<details>
<summary>RHEL / Fedora</summary>

#### x86_64

```bash
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64.rpm
sudo rpm -i nameroute-x86_64.rpm
```

#### ARM64

```bash
curl -LO https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64.rpm
sudo rpm -i nameroute-aarch64.rpm
```

</details>

<details>
<summary>Other platforms</summary>

#### macOS (Apple Silicon)

```bash
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64-apple-darwin -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/
```

#### macOS (Intel)

```bash
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-apple-darwin -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/
```

#### Linux (ARM64)

```bash
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-aarch64-unknown-linux-gnu -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/
```

#### Build from source

```bash
cargo install --git https://github.com/SpreadWorks/name-route
```

</details>


## Quick Start

### 1. Start the daemon

```bash
sudo nameroute
```

All protocol listeners start with zero configuration. You can also run without `sudo`.

<details>
<summary>Running without sudo</summary>

Modern browsers (Chrome, Firefox, Edge, Safari) automatically resolve `*.localhost` to `127.0.0.1`, so browser-based access works without `sudo` — no DNS or `/etc/hosts` changes needed.

`sudo` is only required when accessing `*.localhost` from CLI tools like `curl` or `wget`, or from applications that rely on the OS resolver. These tools need either a DNS server or `/etc/hosts` entries to resolve the hostname.

</details>

### 2. Register a route

```bash
nameroute run http myapp -- next dev
```

Automatically allocates a free port, registers the route, and passes the port to your dev server. If you use Docker, you can also register routes automatically by [adding a label](docs/docker.md) — no port mapping required.

<details>
<summary>Other ways to register routes (Docker / add / config)</summary>

#### Docker

Just add a `name-route` label to your containers. Routes are registered and removed automatically as containers start and stop. This is the easiest approach for Docker-based development.

```yaml
# docker-compose.yml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":3000}]'
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

See [Docker integration](docs/docker.md) for details.

#### `add` command

Manually add or remove routes for already-running services. You specify the port yourself.

```bash
nameroute add http myapp 127.0.0.1:3000
nameroute add postgres myapp 127.0.0.1:5432
```

#### Config / Discovery

Define static routes in a TOML file. Useful when you want the same routes every time you start the daemon.

```toml
# routes.toml
[[routes]]
protocol = "http"
key = "myapp"
backend = "127.0.0.1:3000"
```

```bash
sudo nameroute --config routes.toml
```

With Discovery enabled, name-route automatically finds `.nameroute.toml` files in your project directories. If `key` is omitted, the directory name is used. These files can be checked into Git, making them great for team workflows and project templates.

```toml
# ~/workspace/myapp/.nameroute.toml
[[routes]]
protocol = "http"
backend = "127.0.0.1:3000"
```

</details>

### 3. Access by name

```bash
curl http://myapp.localhost:8080
```

PostgreSQL, MySQL, and SMTP are also accessible by name.

<details>
<summary>Other protocol examples</summary>

```bash
# PostgreSQL — routed by database name
psql -h localhost -p 15432 -d myapp

# MySQL — routed by database name
mysql -h localhost -P 13306 -D myapp

# SMTP — routed by recipient domain
swaks --to user@myapp.localhost --server localhost --port 10025
```

</details>


## HTTPS

The HTTPS listener is enabled by default. name-route supports both **Passthrough** mode (no certs needed) and **Terminate** mode (TLS termination with local certs like mkcert).

```bash
# Passthrough (default) — backend handles TLS
nameroute add https myapp 127.0.0.1:3443
curl https://myapp.localhost:8443
```

See [HTTPS](docs/https.md) for full setup instructions.


## Commands

| Command | Description |
|---------|-------------|
| `nameroute` | Start the daemon (default, equivalent to `nameroute serve`) |
| `nameroute serve` | Start the daemon (for systemd/launchd) |
| `nameroute run` | Auto-allocate port + register route + run command |
| `nameroute add` | Add a route dynamically |
| `nameroute remove` | Remove a route dynamically |
| `nameroute list` | List current routes |
| `nameroute status` | Show daemon status |
| `nameroute reload` | Reload configuration |

```bash
# Add and remove routes
nameroute add http myapp 127.0.0.1:3000
nameroute remove http myapp
```


## Configuration

Configuration is optional. All settings have sensible defaults.

```toml
[general]
log_level = "info"         # trace, debug, info, warn, error

[docker]
enabled = true             # set false to disable Docker integration
poll_interval = 3          # container detection interval (seconds)

[backend]
connect_timeout = 5        # backend connection timeout (seconds)
connect_retries = 3        # connection retry count
idle_timeout = 10          # idle timeout after L7 parsing (seconds)

[listeners.http]
protocol = "http"
bind = "127.0.0.1:8080"

[listeners.https]
protocol = "https"
bind = "127.0.0.1:8443"

[listeners.postgres]
protocol = "postgres"
bind = "127.0.0.1:15432"

[listeners.mysql]
protocol = "mysql"
bind = "127.0.0.1:13306"

[listeners.smtp]
protocol = "smtp"
bind = "127.0.0.1:10025"

[http]
base_domain = "localhost"  # parent domain for subdomains

[dns]
bind = "127.0.0.1:53"     # DNS server address

[smtp]
mailbox_dir = "/var/lib/name-route/mailbox"
max_message_size = 10485760  # 10MB

[discovery]
enabled = true             # set false to disable Discovery
paths = ["~/workspace"]    # parent directories to scan
poll_interval = 3          # scan interval (seconds)

# TLS settings (for terminate mode)
# [tls]
# cert = "cert.pem"
# key = "key.pem"

# Static routes (multiple allowed)
[[routes]]
protocol = "http"
key = "myapp"
backend = "127.0.0.1:3000"
```

See [config.example.toml](config.example.toml) for the full reference.


## Tested with

All combinations of the client libraries below and server versions (PostgreSQL 14–17, MySQL 5.7–8.4) have been verified through integration tests.

| Language | PostgreSQL | MySQL |
|----------|------------|-------|
| C | libpq | libmysqlclient |
| Go | pgx | go-sql-driver/mysql |
| Java | JDBC (postgresql) | mysql-connector-j |
| Node.js | pg | mysql2 |
| PHP | PDO pgsql | PDO mysql |
| Python | psycopg2, psycopg (v3) | PyMySQL, mysqlclient |
| Ruby | pg | mysql2 |
| Rust | tokio-postgres | mysql_async |


## Docs

- [nameroute run](docs/run.md) — `$PORT` substitution, `--detect-port`, `--port-env`
- [Docker integration](docs/docker.md) — Route registration via Docker labels, eliminating `ports:`
- [HTTPS](docs/https.md) — Passthrough and Terminate mode setup
- [Migration guide](docs/migration.md) — Step-by-step guide for migrating existing projects


## License

[MIT](LICENSE)
