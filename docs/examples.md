[日本語](ja/examples.md)

# Configuration examples

This page shows common project layouts and how to configure name-route for each.


## 1. Single app — `nameroute run`

The simplest setup. One dev server, one route.

```
myapp/
├── package.json
└── ...
```

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
npm run dev
# → http://myapp.localhost:8080
```

Route method: **`nameroute run`** — port is allocated automatically and the route is removed when the process exits.


## 2. App + DB — `nameroute run` + Docker label

Host-side Next.js with a PostgreSQL database in Docker.

```
myapp/
├── docker-compose.yml
├── package.json
├── .env
└── ...
```

```yaml
# docker-compose.yml
services:
  db:
    image: postgres
    environment:
      POSTGRES_DB: myapp
      POSTGRES_USER: myapp
      POSTGRES_PASSWORD: myapp
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
# .env
DATABASE_URL=postgres://myapp:myapp@localhost:15432/myapp
```

```bash
docker compose up -d   # Start DB
npm run dev            # Start app

# → http://myapp.localhost:8080  (app)
# → psql -h localhost -p 15432 -d myapp  (DB)
```

Route methods: **`nameroute run`** for the app, **Docker label** for the database. No `ports:` needed in `docker-compose.yml`.


## 3. Monorepo (HTTPS) — `nameroute run`

A monorepo with Next.js frontends and a Wrangler (Cloudflare Workers) API, all served over HTTPS with TLS termination.

```
myproject/
├── apps/
│   ├── frontend/   # Next.js  → https://frontend.myproject.localhost:8443
│   ├── backend/    # Vite     → https://backend.myproject.localhost:8443
│   └── api/        # Wrangler → https://api.myproject.localhost:8443
└── package.json
```

Each app's `package.json`:

```jsonc
// apps/frontend/package.json — Next.js reads PORT automatically
{
  "scripts": {
    "dev": "nameroute run https frontend.myproject --tls-mode terminate -- next dev"
  }
}

// apps/backend/package.json — Vite needs explicit --port
{
  "scripts": {
    "dev": "nameroute run https backend.myproject --tls-mode terminate -- vite dev --host --port '$PORT'"
  }
}

// apps/api/package.json — Wrangler needs sh -c for $PORT expansion
{
  "scripts": {
    "dev": "nameroute run https api.myproject --tls-mode terminate -- sh -c 'CI=1 wrangler dev src/index.ts --ip 0.0.0.0 --port \"$PORT\"'"
  }
}
```

When the first `*.myproject` route is registered, the daemon adds `*.myproject.localhost` to `/etc/nameroute/domains` and logs a certificate regeneration command. Run the following to update the certificate:

```bash
sudo xargs mkcert \
  -key-file /etc/nameroute/key.pem \
  -cert-file /etc/nameroute/cert.pem \
  < /etc/nameroute/domains
sudo systemctl restart nameroute
```

See [HTTPS — Terminate mode](https.md#terminate-mode) for initial certificate setup.

Route method: **`nameroute run`** with multi-level subdomain keys and `--tls-mode terminate`.


## 4. Full Docker — Docker labels only

All services run in Docker containers. No `ports:` needed — name-route routes by name.

```yaml
# docker-compose.yml
services:
  web:
    build: .
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":3000}]'

  api:
    build: ./api
    labels:
      name-route: '[{"protocol":"http","key":"api.myapp","port":8000}]'

  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'

  mail:
    image: mailhog/mailhog
    labels:
      name-route: |
        [
          {"protocol":"http","key":"mail.myapp","port":8025},
          {"protocol":"smtp","key":"myapp","port":1025}
        ]
```

```bash
docker compose up -d

# → http://myapp.localhost:8080       (web)
# → http://api.myapp.localhost:8080   (API)
# → http://mail.myapp.localhost:8080  (mail UI)
# → psql -h localhost -p 15432 -d myapp  (DB)
# → smtp://localhost:10025 key=myapp  (SMTP)
```

Route method: **Docker labels** — name-route polls Docker and registers routes automatically. No port mapping, no conflicts between projects. See [Docker integration](docker.md) for label format details.


## 5. Discovery — `.nameroute.toml`

Automatic route registration for multiple projects under a workspace directory. Each repository contains a `.nameroute.toml` file, and the daemon discovers them automatically.

```
~/workspace/
├── blog/
│   ├── .nameroute.toml
│   └── ...
├── shop/
│   ├── .nameroute.toml
│   └── ...
└── api-server/
    ├── .nameroute.toml
    └── ...
```

Each project's `.nameroute.toml`:

```toml
# ~/workspace/blog/.nameroute.toml
[[routes]]
protocol = "http"
key = "blog"
backend = "127.0.0.1:3000"
```

```toml
# ~/workspace/shop/.nameroute.toml
[[routes]]
protocol = "http"
key = "shop"
backend = "127.0.0.1:3001"
```

```toml
# ~/workspace/api-server/.nameroute.toml
[[routes]]
protocol = "http"
key = "api"
backend = "127.0.0.1:8000"
```

Daemon configuration:

```toml
[discovery]
enabled = true
paths = ["~/workspace"]
poll_interval = 3
```

```bash
# → http://blog.localhost:8080
# → http://shop.localhost:8080
# → http://api.localhost:8080
```

Route method: **Discovery** — the daemon scans the configured directories for `.nameroute.toml` files. Routes are registered and removed as files appear and disappear. The `.nameroute.toml` file can be committed to version control so every developer gets the same routing setup.


## Route methods summary

| Method | How it works | Best for |
|--------|-------------|----------|
| `nameroute run` | Wraps your dev command, allocates a port, registers a route | Host-side dev servers |
| Docker label | `name-route` label on containers, auto-discovered | Dockerized services |
| Discovery | `.nameroute.toml` in project directories, auto-scanned | Multi-project workspaces |
| Static route | `[[routes]]` in daemon config file | Always-on services |
| `nameroute add` | CLI command to register a route manually | One-off / scripting |
