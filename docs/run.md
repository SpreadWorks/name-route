[日本語](ja/run.md)

# nameroute run

`nameroute run` starts your dev server and registers a route in a single command. It automatically allocates a free port, passes it to the child process, and registers the route with the daemon. When the child process exits, the route is automatically removed.


## Basic usage

```bash
nameroute run <protocol> <key> -- <command...>
```

### Aliases

Repeat `--alias` to expose the same child and backend port under additional
route keys. The keys are reserved atomically: a conflict leaves no partial
aliases behind.

```bash
nameroute run http app --alias api --alias admin -- next dev
```

All of these routes are owned by this invocation and are removed together when
it exits. A later manual or discovered route is never removed by stale cleanup.

```bash
# Start a Next.js dev server
nameroute run http myapp -- next dev

# → A free port is allocated and passed via the PORT env var
# → Accessible at http://myapp.localhost:8080
# → Ctrl+C stops the server and removes the route
```


## Port passing

### PORT environment variable (default)

`nameroute run` finds a free port and passes it to the child process as the `PORT` environment variable. Most frameworks — Next.js, Vite, Rails, and others — pick up `PORT` automatically.

```bash
nameroute run http myapp -- next dev
# next dev receives PORT=XXXXX and starts on that port
```

### $PORT argument substitution

Any `$PORT` in command arguments is replaced with the allocated port number. This is useful for commands that don't read the `PORT` environment variable.

```bash
nameroute run http myapp -- python3 -m http.server '$PORT'
# → Expands to: python3 -m http.server 12345
```

> **Note:** Use single quotes around `$PORT` to prevent your shell from expanding it.

### --port-env option

Pass the port via an additional environment variable name. Both `PORT` and the specified name are set.

```bash
nameroute run http api --port-env DEV_API_PORT -- next dev
# → Both PORT=XXXXX and DEV_API_PORT=XXXXX are set
```

Useful when your service expects a custom environment variable name for the port.


## --detect-port mode

For commands that don't read `PORT` or that choose their own port, `--detect-port` automatically detects the port from stdout/stderr output.

```bash
nameroute run http myapp --detect-port -- python3 -m http.server 0
# → Detects "http://0.0.0.0:XXXXX" from stdout and registers the route
```

Detected patterns:
```
http://localhost:<port>
http://127.0.0.1:<port>
http://0.0.0.0:<port>
https://localhost:<port>
```

### FORCE_COLOR

In `--detect-port` mode, stdout/stderr are piped, which may cause child processes to disable colored output. nameroute automatically sets `FORCE_COLOR=1` to preserve colors.

## Shutdown behavior

On Linux and macOS, the child runs in its own process group. SIGINT and SIGTERM
received by `nameroute run` are forwarded to that group; after a short grace
period an unresponsive group is killed. Route cleanup is owner-scoped and has a
timeout, so cleanup cannot accidentally remove a replacement route or wait
forever. A port listener is held until immediately before spawning the child;
passing an arbitrary listener to every possible child command is not portable,
so the final bind-to-exec interval cannot be reserved completely.

For correctness across delayed management connections, the daemon retains a
small owner UUID tombstone for each completed `run` until the daemon restarts.
This makes a late registration a no-op rather than resurrecting a route; memory
therefore grows with completed runs and is reclaimed on daemon restart.

Descendants that deliberately leave the managed process group or create a new
session are outside this guarantee. When run interactively, terminal ownership
is handed to the child group and restored before route cleanup. A background
`nameroute run ... &` job does not take terminal ownership from the shell.


## HTTPS with --tls-mode

Use `--tls-mode terminate` to run HTTPS routes where name-route handles TLS termination. The backend runs plain HTTP.

```bash
nameroute run https myapp --tls-mode terminate -- next dev --port '$PORT'
```

This requires a `[tls]` section in the daemon's config with paths to the certificate and key (e.g. `/etc/nameroute/cert.pem` and `/etc/nameroute/key.pem`). See [HTTPS](https.md) for setup.

Without `--tls-mode`, passthrough mode is used and the backend must serve TLS itself.


## Signal handling

When you press Ctrl+C, nameroute forwards SIGINT to the child process. This is identical to normal Ctrl+C behavior, so dev servers in Node.js, Python, and other runtimes can perform a graceful shutdown. After the child process exits, nameroute automatically removes the route from the daemon.


## package.json example

```json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev",
    "dev:api": "nameroute run http api.myapp -- node server.js"
  }
}
```

Multi-level subdomains like `api.myapp` give you URLs like `http://api.myapp.localhost:8080`.

For more examples including monorepo and HTTPS setups, see [Configuration examples](examples.md).


## docker-compose.yml example

Use this pattern when your app runs on the host but your database runs in Docker.

```yaml
# docker-compose.yml (DB only)
services:
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```json
// package.json
{
  "scripts": {
    "dev": "nameroute run http myapp -- next dev"
  }
}
```

```bash
docker compose up -d   # Start DB
npm run dev            # Start app
# → http://myapp.localhost:8080 for the app
# → psql -h localhost -p 15432 -d myapp for the DB
```


## Route listing

```bash
nameroute list
```

```
PROTOCOL     KEY                  BACKEND                  SOURCE   HEALTH     URL
http         myapp                127.0.0.1:43210          run      healthy    http://myapp.localhost:8080
postgres     myapp                172.17.0.2:5432          docker   healthy
```

Routes include a HEALTH column showing backend connectivity status.
