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
curl -L https://github.com/SpreadWorks/name-route/releases/latest/download/nameroute-x86_64-unknown-linux-gnu -o nameroute
chmod +x nameroute
sudo mv nameroute /usr/local/bin/

# Start (sudo is needed for DNS and /etc/hosts)
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

Yes. Routing works fine without `sudo`. Modern browsers (Chrome, Firefox, Edge, Safari) automatically resolve `*.localhost` to `127.0.0.1`, so browser-based access doesn't need DNS or `/etc/hosts` entries.

`sudo` is only needed for CLI tools like `curl` and `wget`, or applications that rely on the OS resolver. These tools need either a DNS server or `/etc/hosts` entries to resolve `*.localhost` hostnames.

### Can I use it with Supabase?

Yes. Add name-route labels to your Supabase Docker containers and point `API_EXTERNAL_URL` and `SITE_URL` to the name-route URLs. This eliminates manual port management for Supabase.
