[日本語](ja/docker.md)

# Docker integration

name-route automatically discovers routing information from Docker container labels. This lets you drop the `ports:` section from your `docker-compose.yml` and stop managing port numbers entirely.


## Basic label configuration

```yaml
services:
  web:
    image: nginx
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
```

name-route combines the container's IP address with the port specified in the label and registers a route automatically.


## Removing ports:

Traditional `docker-compose.yml`:

```yaml
services:
  web:
    image: nginx
    ports:
      - "3000:80"       # ← You have to remember this port
  db:
    image: postgres
    ports:
      - "5432:5432"     # ← May conflict with other projects
```

With name-route:

```yaml
services:
  web:
    image: nginx
    # No ports: needed!
    labels:
      name-route: '[{"protocol":"http","key":"myapp","port":80}]'
  db:
    image: postgres
    # No ports: needed!
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

**Benefits:**
- No port conflicts — name-route routes by name, not port number
- Run multiple projects simultaneously
- Cleaner `docker-compose.yml`


## Label format

The label value is a JSON array, allowing multiple routes per container.

```json
[
  {"protocol": "http", "key": "myapp", "port": 80},
  {"protocol": "postgres", "key": "myapp"}
]
```

| Field | Required | Description |
|-------|----------|-------------|
| `protocol` | Yes | One of `http`, `https`, `postgres`, `mysql`, `smtp` |
| `key` | No | Routing key. Defaults to the container name |
| `port` | No | Container port. Defaults to the protocol's standard port |

Default ports:
- HTTP: 80
- HTTPS: 443
- PostgreSQL: 5432
- MySQL: 3306
- SMTP: 25


## Labels with docker run

You can specify labels with `docker run` even without a `docker-compose.yml`.

```bash
# Single route
docker run --label 'name-route=[{"protocol":"http","key":"myapp","port":80}]' nginx

# Multiple routes
docker run --label 'name-route=[{"protocol":"http","key":"myapp","port":3000},{"protocol":"postgres","key":"myapp"}]' myapp
```

This also works with `docker compose run`:

```bash
docker compose run --label 'name-route=[{"protocol":"http","key":"myapp","port":80}]' web
```

> **Note:** Labels specified via `docker compose run` take precedence over those in `docker-compose.yml`. This is handy for temporarily registering a route under a different key.


## Multiple routes

To expose multiple protocols from a single container:

```yaml
services:
  app:
    image: myapp
    labels:
      name-route: |
        [
          {"protocol":"http","key":"myapp","port":3000},
          {"protocol":"postgres","key":"myapp","port":5432}
        ]
```


## Container lifecycle tracking

name-route periodically polls Docker (every 3 seconds by default) to detect container changes.

- Container starts → route is registered
- Container stops → route is removed

The polling interval is configurable:

```toml
[docker]
enabled = true
poll_interval = 3  # seconds
```


## Mixing with non-Docker services

You can combine Docker containers with host-side dev servers.

```yaml
# docker-compose.yml — DB only
services:
  db:
    image: postgres
    labels:
      name-route: '[{"protocol":"postgres","key":"myapp"}]'
```

```bash
# Run the app on the host
nameroute run http myapp -- next dev
```

```bash
# Access both by name
curl http://myapp.localhost:8080       # → Next.js on the host
psql -h localhost -p 15432 -d myapp    # → PostgreSQL in Docker
```
