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
mkcert -install

# Generate a wildcard certificate
mkcert -key-file key.pem -cert-file cert.pem "*.localhost"
```

### 2. Configure

Add a TLS section and set `tls_mode = "terminate"` on the route:

```toml
[tls]
cert = "cert.pem"
key = "key.pem"

[[routes]]
protocol = "https"
key = "myapp"
backend = "127.0.0.1:3000"
tls_mode = "terminate"
```

```bash
sudo nameroute --config config.toml
```

### 3. Access

```bash
# name-route terminates TLS and forwards HTTP to the backend
curl https://myapp.localhost:8443
```
