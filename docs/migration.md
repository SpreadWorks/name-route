[日本語](ja/migration.md)

# Migration guide

This guide walks you through migrating an existing project to name-route.


## Overview

The goal is to eliminate hardcoded port numbers and switch to name-based routing.

**Before:**
```
http://localhost:3000     → Frontend
http://localhost:4000     → API
postgresql://localhost:5432/mydb
```

**After:**
```
http://myapp.localhost:8080       → Frontend
http://api.myapp.localhost:8080   → API
postgresql://localhost:15432/mydb
```


## Step 1: Install & start

```bash
# Install (see README for all platforms)
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-musl -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/

# Start (sudo is needed for /etc/hosts management)
sudo nameroute
```


## Step 2: Remove hardcoded ports

### package.json dev scripts

**Before:**
```json
{
  "scripts": {
    "dev": "next dev -p 3000"
  }
}
```

**After:**
```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

`nameroute run` allocates a free port automatically and passes it via the `PORT` environment variable. Next.js, Vite, Remix, and others pick this up out of the box.

### Custom environment variables

If your project uses a custom environment variable for the port (e.g. `DEV_API_PORT`):

```json
{
  "scripts": {
    "dev:api": "nameroute run http api --port-env DEV_API_PORT -- node server.js"
  }
}
```

### docker-compose.yml

**Before:**
```yaml
services:
  web:
    image: nginx
    ports:
      - "3000:80"
  db:
    image: postgres
    ports:
      - "5432:5432"
```

**After:**
```yaml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

Remove `ports:` and add a `name-route` label instead.


## Step 3: Update connection URLs

### Frontend API calls

**Before:**
```javascript
// .env.local
API_URL=http://localhost:4000
```

**After:**
```javascript
// .env.local
API_URL=http://api.myapp.localhost:8080
```

### Database connection

**Before:**
```
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb
```

**After:**
```
DATABASE_URL=postgresql://user:pass@localhost:15432/mydb
```

The name-route PostgreSQL listener defaults to port `15432`. The database name (`mydb`) is used as the routing key.

### CORS

If your API server has CORS configured, allow requests from `*.localhost`.

```javascript
// Express
app.use(cors({
  origin: /\.localhost(:\d+)?$/
}));
```

```python
# Django
CORS_ALLOWED_ORIGIN_REGEXES = [
    r"^https?://.*\.localhost(:\d+)?$",
]
```

```ruby
# Rails
Rails.application.config.hosts << /.*\.localhost/
```


## Step 4: Multi-level subdomains

For monorepos or multi-service architectures, multi-level subdomains keep things organized.

```bash
# Monorepo setup
nameroute run http myapp -- next dev          # → http://myapp.localhost:8080
nameroute run http api.myapp -- node api.js   # → http://api.myapp.localhost:8080
nameroute run http admin.myapp -- next dev    # → http://admin.myapp.localhost:8080
```

Use dots in the routing key to express logical hierarchy.


## Step 5: Verify routes

```bash
nameroute list
```

```
PROTOCOL     KEY                  BACKEND                  SOURCE   URL
http         myapp                127.0.0.1:43210          run      http://myapp.localhost:8080
http         api.myapp            127.0.0.1:43211          run      http://api.myapp.localhost:8080
postgres     myapp                172.17.0.2:5432          docker
```


## FAQ

### Do I need to change Docker network settings?

No. name-route connects directly to container IP addresses, so your Docker network configuration stays the same. The only change is removing `ports:` entries that are no longer needed.

### Can I run multiple projects simultaneously?

Yes — this is one of name-route's primary benefits. As long as each project uses a different routing key, there are no port conflicts.

```bash
# Project A
nameroute run http project-a -- next dev

# Project B (separate terminal)
nameroute run http project-b -- next dev
```

### Can I use it without sudo?

Yes. Routing works fine without `sudo`. Modern browsers (Chrome, Firefox, Edge, Safari) automatically resolve `*.localhost` to `127.0.0.1`, so browser-based access doesn't need `/etc/hosts` entries.

`sudo` is only needed for CLI tools like `curl` and `wget`, or server-to-server communication that relies on the OS resolver. These tools need `/etc/hosts` entries to resolve `*.localhost` hostnames.

### Can I use it with Supabase?

Yes. Add name-route labels to your Supabase Docker containers and point `API_EXTERNAL_URL` and `SITE_URL` to the name-route URLs. This eliminates manual port management for Supabase.


---

## AI agent instructions

This section provides instructions for AI agents performing name-route migration on existing projects.

To use this guide, give the AI agent the following prompt with the raw URL of this file:

```
Migrate this project to use name-route by following the instructions at:
https://raw.githubusercontent.com/SpreadWorks/name-route/refs/heads/main/docs/migration.md

The project directory is: /path/to/project
```

### Prerequisites

- `nameroute` is already installed on the system.
- The daemon is managed with systemctl (`sudo systemctl start nameroute`). If systemctl is not available, ask the user how they start and stop nameroute before proceeding.
- Verify the daemon is running with `nameroute status`.

### Default listener ports

| Protocol   | Listen address      | Routing key source       |
|------------|---------------------|--------------------------|
| HTTP       | `127.0.0.1:8080`    | Subdomain of Host header |
| HTTPS      | `127.0.0.1:8443`    | SNI (TLS Server Name)    |
| PostgreSQL | `127.0.0.1:15432`   | Database name            |
| MySQL      | `127.0.0.1:13306`   | Database name            |
| SMTP       | `127.0.0.1:10025`   | Recipient domain         |

### Migration patterns

#### Pattern 1: Host-side dev server (`nameroute run`)

Wrap the start command. `nameroute run` allocates a free port and passes it via the `PORT` environment variable. Most frameworks (Next.js, Vite, Rails, etc.) read `PORT` automatically.

```json
{ "scripts": { "dev": "nameroute run http myapp -- next dev" } }
```

Variants:
- **`$PORT` substitution** (for commands that don't read `PORT`): `nameroute run http myapp -- python3 -m http.server '$PORT'`
- **`--port-env`** (additional env var): `nameroute run http myapp --port-env DEV_API_PORT -- node server.js`
- **`--detect-port`** (auto-detect from stdout/stderr): `nameroute run http myapp --detect-port -- some-server`

#### Pattern 2: Docker containers (labels)

Replace `ports:` with a `name-route` label. The label value is a JSON array:

```yaml
labels:
  name-route: '[{"protocol":"http","key":"myapp","port":80}]'
```

| Field      | Required | Default                         |
|------------|----------|---------------------------------|
| `protocol` | Yes      |                                 |
| `key`      | No       | Container name                  |
| `port`     | No       | HTTP=80, HTTPS=443, PG=5432, MySQL=3306, SMTP=25 |

#### Pattern 3: Host app + Docker DB (mixed)

Combine Pattern 1 for the app and Pattern 2 for the database. See the examples in the main migration steps above.

#### Pattern 4: Multi-level subdomains (monorepo)

Use dotted keys: `frontend.myproject`, `api.myproject` → `http://frontend.myproject.localhost:8080`, `http://api.myproject.localhost:8080`.

#### Pattern 5: HTTPS with TLS termination

Use `--tls-mode terminate`. Requires `[tls]` cert/key in the daemon config. For multi-level subdomains, regenerate certificates from `/etc/nameroute/domains`.

### DATABASE_URL rules

- **Host to DB**: Change port to the nameroute listener port (15432 for PostgreSQL, 13306 for MySQL). The database name becomes the routing key.
- **Docker internal**: If a container connects via Docker network using the service name as hostname (e.g., `mysql://dbuser:secret@mysql/eccubedb`), do NOT change it.
- **BaaS (Supabase, Firebase, etc.)**: If the project uses a BaaS with its own CLI (e.g., `npx supabase start`) and connects via SDK (`SUPABASE_URL`, `SUPABASE_ANON_KEY`), do not change the BaaS settings. Only apply `nameroute run` to the HTTP app servers.

### Migration checklist

1. **Confirm the daemon is running** — `nameroute status`
2. **Identify services** — list all dev servers and databases the project uses
3. **Choose a routing key** — typically the project name (e.g., `myapp`)
4. **For host-side dev servers** — wrap the start command with `nameroute run http <key> -- <command>`
5. **For Docker services** — replace `ports:` with `name-route` labels
6. **Update DATABASE_URL** — change port to the nameroute listener port (15432, 13306). See DATABASE_URL rules above for exceptions.
7. **Update any hardcoded URLs** — change `http://localhost:<port>` to `http://<key>.localhost:8080`. Also update CORS settings (`Access-Control-Allow-Origin`, `CORS_ALLOW_ORIGIN`, etc.) and other origin-dependent configuration (`TRUSTED_HOSTS`, CSP headers) to allow the new `*.localhost` origins.
8. **If HTTPS is needed** — use `--tls-mode terminate` and confirm `[tls]` is configured in the daemon
9. **Test** — run `nameroute list` to verify routes, then access the service by name
