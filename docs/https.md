[日本語](ja/https.md)

# HTTPS

name-route enables the HTTPS listener by default. There are two modes: Passthrough and Terminate.


## Passthrough mode (default)

TLS traffic is forwarded directly to the backend. The backend handles TLS termination, so name-route needs no certificates. This works out of the box with zero configuration.

```bash
# When the backend handles TLS itself (e.g. Next.js --experimental-https)
nameroute add https myapp 127.0.0.1:3443
curl https://myapp.localhost:8443
```

Also works with `nameroute run`:

```bash
nameroute run https myapp -- next dev --experimental-https
```


## Terminate mode

name-route terminates TLS and forwards plain HTTP to the backend. This is useful when you want HTTPS access without modifying your backend. Requires local certificates (e.g. from mkcert).

### 1. Generate certificates

```bash
# Install mkcert (one-time setup)
brew install mkcert  # macOS
# apt install mkcert  # Linux

# Install the local CA
sudo mkcert -install

# Generate a wildcard certificate
sudo mkdir -p /etc/nameroute
sudo mkcert -key-file /etc/nameroute/key.pem \
            -cert-file /etc/nameroute/cert.pem \
            "*.localhost"
```

> **Why sudo?** mkcert stores its CA per user (`~/.local/share/mkcert/`). If you run `mkcert -install` as a regular user but generate certificates with `sudo mkcert`, the CA that signed the certificate won't match the one trusted by the browser. Always use `sudo` for all mkcert commands to keep the CA consistent.

> **Tip:** The daemon automatically tracks which wildcard patterns are needed in `/etc/nameroute/domains`.
> When a multi-level subdomain route is registered (e.g. `frontend.echub`), the daemon appends `*.echub.localhost` to the file and logs a certificate regeneration command.
>
> To regenerate the certificate covering all patterns:
> ```bash
> sudo xargs mkcert \
>   -key-file /etc/nameroute/key.pem \
>   -cert-file /etc/nameroute/cert.pem \
>   < /etc/nameroute/domains
> sudo systemctl restart nameroute
> ```

### 2. Configure

Add a TLS section to your config file:

```toml
[tls]
cert = "/etc/nameroute/cert.pem"
key = "/etc/nameroute/key.pem"
```

### 3. Use with `nameroute run`

Pass `--tls-mode terminate` so name-route handles TLS. The backend runs plain HTTP — no `--experimental-https` or similar flags needed.

```bash
nameroute run https myapp --tls-mode terminate -- next dev --port '$PORT'
```

```
curl ──tls──▶ nameroute:8443 ──http──▶ next:$PORT (HTTP)
```

#### package.json example

```json
{
  "scripts": {
    "dev": "nameroute run https myapp --tls-mode terminate -- next dev"
  }
}
```

### 4. Use with static routes

Set `tls_mode = "terminate"` on the route:

```toml
[tls]
cert = "/etc/nameroute/cert.pem"
key = "/etc/nameroute/key.pem"

[[routes]]
protocol = "https"
key = "myapp"
backend = "127.0.0.1:3000"
tls_mode = "terminate"
```

```bash
sudo nameroute --config config.toml
curl https://myapp.localhost:8443
```


## Passthrough vs Terminate

| | Passthrough (default) | Terminate |
|---|---|---|
| TLS handled by | Backend | name-route |
| Certificates | Backend manages | name-route config (`[tls]`) |
| Backend protocol | HTTPS | HTTP |
| Use case | Backend already serves TLS | Add HTTPS without changing backend |
| `nameroute run` | `nameroute run https myapp -- next dev --experimental-https` | `nameroute run https myapp --tls-mode terminate -- next dev` |

> **Note:** In passthrough mode, the certificate's domain must match the routing key (e.g. `myapp.localhost`). Dev servers like Next.js `--experimental-https` typically generate certificates for `localhost` only, which causes domain mismatch warnings. Use terminate mode to avoid this.
