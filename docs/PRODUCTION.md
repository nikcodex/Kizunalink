# Production Deployment Guide

Everything needed to run KizunaLink as a long-lived, public-facing node.

---

## 1. Minimum production checklist

- [ ] `config.toml` created from `config.example.toml` with a **unique**
      `server.authorization` password
- [ ] Secrets via env vars (`KIZUNA_AUTHORIZATION`) or Docker secrets — not
      baked into the file
- [ ] `rate_limit_per_minute` left at 2000 (or tuned) — `0` disables it, only
      do that behind your own proxy
- [ ] TLS enabled (`[server.tls]`) **or** TLS terminated at a reverse proxy —
      never send the authorization password in cleartext over the internet
- [ ] `[metrics.prometheus] enabled = true` if you scrape metrics (endpoint is
      auth-protected; scrape with the node password)
- [ ] Log rotation configured (`[logging.file] rotate_daily = true`) and the
      log directory writable by the `kizunalink` user
- [ ] `/health` wired into your orchestrator (Docker `HEALTHCHECK`, k8s probe)
- [ ] Provider credentials configured for the sources you use — see
      [CREDENTIALS.md](./CREDENTIALS.md)

---

## 2. Docker (recommended)

`docker-compose.yml` in the repo root is production-hardened (non-root user,
`cap_drop: ALL`, `no-new-privileges`, read-only rootfs). One fix matters:

```yaml
volumes:
  - ./config.toml:/app/config.toml:ro   # must be /app/, the WORKDIR
```

**Logs**: the default log path is relative (`./logs/`), which is inside the
read-only rootfs. Either point logs at the tmpfs:

```toml
[logging.file]
path = "/tmp/logs/kizunalink.log"
rotate_daily = true
max_files = 7
```

…or mount a writable volume:

```yaml
volumes:
  - ./logs:/app/logs
```

**Secrets** (avoid passwords on disk):

```yaml
services:
  kizunalink:
    environment:
      KIZUNA_AUTHORIZATION: "${KIZUNA_AUTHORIZATION}"   # from .env / secrets
      KIZUNA_PORT: "2333"
```

Bring up and verify:

```bash
docker compose up -d
curl -s http://localhost:2333/health
# → {"status":"ok",...}
```

### Healthchecks

Add to compose:

```yaml
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://localhost:2333/health"]
      interval: 30s
      timeout: 5s
      retries: 3
```

(The runtime image is Debian-slim; if `curl` is missing use the binary image's
built-in `wget` or install curl in a derived image.)

---

## 3. Reverse proxy (Caddy / nginx)

Terminate TLS at the proxy instead of in-process if you prefer:

**Caddy** (simplest, automatic certificates):

```
lava.example.com {
    reverse_proxy 127.0.0.1:2333
}
```

**nginx** (WebSockets need upgrade headers):

```nginx
server {
    listen 443 ssl;
    server_name lava.example.com;
    # ssl_certificate / key ...

    location / {
        proxy_pass http://127.0.0.1:2333;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 360s;   # keep WS sessions alive
    }
}
```

Then set `server.tls.enabled = false` in KizunaLink (the proxy owns TLS).

Behind a proxy the per-IP rate limiter sees the proxy's IP. If all your bot
traffic comes through one reverse proxy you may want a higher
`rate_limit_per_minute` or `0` plus rate limiting at the proxy.

---

## 4. Kubernetes

Minimal deployment shape:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: kizunalink }
spec:
  replicas: 1
  selector: { matchLabels: { app: kizunalink } }
  template:
    metadata: { labels: { app: kizunalink } }
    spec:
      containers:
        - name: kizunalink
          image: ghcr.io/nikcodex/kizunalink:latest
          ports: [{ containerPort: 2333 }]
          env:
            - name: KIZUNA_AUTHORIZATION
              valueFrom:
                secretKeyRef: { name: kizuna, key: password }
          readinessProbe:
            httpGet: { path: /health, port: 2333 }
          livenessProbe:
            httpGet: { path: /health, port: 2333 }
          securityContext:
            runAsNonRoot: true
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
```

State is in-memory (players/sessions) — a restart drops active playback, so
keep `replicas: 1` per node and let clients reconnect via session resumption
(60s default window; clients can extend to 600s).

---

## 5. Build metadata (`/v4/info`)

CI should stamp build info so operators can identify deployments:

```bash
GIT_BRANCH=$(git rev-parse --abbrev-ref HEAD) \
GIT_COMMIT=$(git rev-parse --short HEAD) \
GIT_COMMIT_TIME=$(git log -1 --format=%ct000) \
BUILD_TIME=$(date +%s%3N) \
cargo build --release
```

Without these, `/v4/info` reports `unknown`/`0` (functional, just less useful).

---

## 6. Capacity planning (measured on this codebase)

- Idle RSS ≈ 20–30 MB, startup < 1 s
- 128 concurrent players across 8 sessions: sustained REST churn with no leaks
  (see the soak test, `cargo test -p kizuna-server -- --ignored soak`)
- Per playing player: one 50 Hz Opus encode loop + mixer — budget roughly one
  core per 30–50 simultaneous playing players (heavily filter-dependent)

---

## 7. Troubleshooting quick hits

| Symptom | Likely cause |
|---|---|
| Bot connects but `ytsearch:` returns `empty` | YouTube PO token expired — rotate it |
| `Connection refused` on 2333 | port mismatch / firewall (`ufw allow 2333`) |
| 401 on REST | missing `Authorization` header |
| 403 on REST | wrong password (deliberate 403, matches official Lavalink) |
| 429 with `retry-after` | rate limiter — raise `rate_limit_per_minute` |
| Metrics 404 | enable `[metrics.prometheus] enabled = true` |
| TLS server won't start | cert/key path wrong — it fails fast by design |
| Container exits immediately | config not found — mount at `/app/config.toml` |

---

## 8. Upgrades

Track protocol-relevant releases; test with a canary bot before rolling:

```bash
git pull
cargo build --release --locked
sudo systemctl restart kizunalink   # or: docker compose up -d --build
```

`/version` (with auth) and `/v4/info` tell you what's running.
