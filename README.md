<h1 align="center">KizunaLink</h1>

<p align="center">
  High-performance audio node for Discord bots, written in Rust.
</p>

<p align="center">
  <a href="https://github.com/nikcodex/Kizunalink/releases"><img src="https://img.shields.io/github/v/release/nikcodex/Kizunalink?style=for-the-badge&color=orange&logo=github" alt="Release"></a>
  <a href="https://github.com/nikcodex/Kizunalink/actions"><img src="https://img.shields.io/github/actions/workflow/status/nikcodex/Kizunalink/ci.yml?style=for-the-badge&logo=githubactions&logoColor=white" alt="Build Status"></a>
  <br>
  <img src="https://img.shields.io/badge/Language-Rust-orange?style=for-the-badge&logo=rust" alt="Language">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey?style=for-the-badge" alt="Platform">
  <img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="License">
</p>

---

KizunaLink is a standalone audio sending node for Discord bots. It implements the **Lavalink v4 protocol**, making it a drop-in replacement for [Lavalink](https://github.com/lavalink-devs/Lavalink) with native Rust performance.

## Features

- **30+ Audio Sources** — YouTube, Spotify, Deezer, SoundCloud, Apple Music, Tidal, Pandora, and more
- **24 Audio Filters** — Equalizer, Karaoke, Reverb, Chorus, Flanger, Phaser, Timescale, Tremolo, etc.
- **DAVE Encryption** — Discord Audio/Video End-to-End encryption support
- **Lyrics Support** — 8 providers (YouTube Music, LRCLib, Genius, Musixmatch, Netease, etc.)
- **Advanced Audio Pipeline** — Multi-track mixer, 3 resampling algorithms (linear, hermite, sinc), buffer pool
- **Prometheus Monitoring** — Built-in metrics endpoint for observability
- **Session Resumption** — Survives WebSocket disconnects with configurable timeout
- **Route Planner** — IP rotation for avoiding rate limits on source APIs

## Architecture

```
┌──────────────┐    WebSocket/REST    ┌──────────────────────────────────────┐
│  Discord Bot │ ◄──────────────────► │           KizunaLink Server          │
│  (any lib)   │    Lavalink v4       │  ┌────────┐  ┌──────────┐  ┌──────┐ │
└──────────────┘                      │  │ Router │  │ Sessions │  │ Auth │ │
                                      │  └───┬────┘  └────┬─────┘  └──────┘ │
                                      │      │            │                  │
                                      │  ┌───▼────────────▼──────────────┐   │
                                      │  │        Player Manager         │   │
                                      │  │  ┌────────┐ ┌─────────────┐  │   │
                                      │  │  │ Source  │ │   Player    │  │   │
                                      │  │  │ Manager │ │  Context    │  │   │
                                      │  │  └────┬───┘ └──────┬──────┘  │   │
                                      │  └───────┼────────────┼─────────┘   │
                                      │          │            │             │
                                      │  ┌───────▼────┐ ┌────▼──────────┐  │
                                      │  │  27 Source  │ │ Audio Engine  │  │
                                      │  │  Plugins    │ │ ┌───────────┐ │  │
                                      │  │  (YouTube,  │ │ │  Decoder  │ │  │
                                      │  │   Spotify,  │ │ │(Symphonia)│ │  │
                                      │  │   Deezer..) │ │ └─────┬─────┘ │  │
                                      │  └────────────┘ │ │ ┌────▼─────┐ │  │
                                      │                │ │ │ Resampler │ │  │
                                      │                │ │ └────┬─────┘ │  │
                                      │                │ │ ┌────▼─────┐ │  │
                                      │                │ │ │ Filters  │ │  │
                                      │                │ │ │ (24 DSP) │ │  │
                                      │                │ │ └────┬─────┘ │  │
                                      │                │ │ ┌────▼─────┐ │  │
                                      │                │ │ │  Mixer   │ │  │
                                      │                │ │ └────┬─────┘ │  │
                                      │                │ │ ┌────▼─────┐ │  │
                                      │                │ │ │  Opus    │ │  │
                                      │                │ │ │ Encoder  │ │  │
                                      │                │ │ └────┬─────┘ │  │
                                      │                │ └──────┼───────┘  │
                                      │  ┌─────────────┐ ┌─────▼───────┐  │
                                      │  │   Gateway   │ │  UDP Send   │  │
                                      │  │ (Discord WS)│ │ (Discord)   │  │
                                      │  └─────────────┘ └─────────────┘  │
                                      └──────────────────────────────────────┘
```

## Quick Start

### Docker (Recommended)

```bash
# Copy and edit config
cp config.example.toml config.toml
# Edit config.toml with your settings

docker compose up -d
```

### From Source

**Prerequisites:** Rust 1.88+ and `libopus-dev`

```bash
# Ubuntu/Debian
sudo apt-get install -y libopus-dev cmake pkg-config libclang-dev clang

# macOS
brew install opus cmake pkg-config

# Build
git clone https://github.com/nikcodex/Kizunalink.git
cd KizunaLink
cargo build --release
./target/release/kizuna-server
```

## Configuration

```toml
[server]
address = "0.0.0.0"
port = 2333
authorization = "youshallnotpass"   # Change this!

[sources.youtube]
enabled = true

[sources.spotify]
enabled = true
clientId = ""
clientSecret = ""

[filters]
volume = true
equalizer = true
karaoke = true
timescale = true
# ... see config.example.toml for all 24 filters
```

On startup (`AppConfig::load`) KizunaLink reads `config.toml` in the working
directory, falling back to `config.example.toml` if absent. It then applies
`KIZUNA_*` environment-var overrides and validates the result, failing fast on
bad values. See [TLS](#tls-httpswss), [Environment Variable
Overrides](#environment-variable-overrides), and [Prometheus
Metrics](#prometheus-metrics) below.

## Supported Sources

| Source | Search | ISRC | Lyrics | Status |
|--------|--------|------|--------|--------|
| YouTube | ✅ | — | ✅ | Stable |
| Spotify | ✅ | ✅ | — | Stable |
| Deezer | ✅ | ✅ | ✅ | Stable |
| SoundCloud | ✅ | — | — | Stable |
| Apple Music | ✅ | ✅ | — | Stable |
| Tidal | ✅ | ✅ | — | Stable |
| Bandcamp | ✅ | — | — | Disabled by default |
| Pandora | ✅ | — | — | Stable |
| JioSaavn | ✅ | — | — | Stable |
| Gaana | ✅ | — | — | Stable |
| Mixcloud | ✅ | — | — | Stable |
| Netease | ✅ | — | ✅ | Stable |
| VK Music | ✅ | — | — | Stable |
| Yandex Music | ✅ | — | — | Stable |
| Audiomack | ✅ | — | — | Stable |
| Audius | ✅ | — | — | Stable |
| Reddit | ✅ | — | — | Stable |
| Twitch | ✅ | — | — | Stable |
| HTTP URLs | ✅ | — | — | Stable |
| Local Files | ✅ | — | — | Stable |
| Last.fm | — | — | — | Mirror |
| Shazam | ✅ | — | — | Disabled by default |
| Anghami | ✅ | — | — | Stable |
| Qobuz | ✅ | ✅ | — | Stable |
| Amazon Music | ✅ | ✅ | — | Stable |
| Flowery TTS | ✅ | — | — | Stable |
| Google TTS | ✅ | — | — | Stable |

> **Bandcamp and Shazam are disabled by default.** Both were verified broken at the
> provider level during testing — Bandcamp's `/search` serves a CSP anti-bot
> challenge, and Shazam's catalog API returns `403` to datacenter traffic. The code
> is kept for a future fix, but a default deployment does not ship known-broken
> search. To enable anyway, set `enabled = true` under `[sources.bandcamp]` /
> `[sources.shazam]` in `config.example.toml` (only works in networks where those
> providers don't block you).

## Bot Integration

KizunaLink is compatible with any Lavalink v4 client library:

**discord.js (v14+)**
```js
const { LavalinkManager } = require("lavalink-client");
const manager = new LavalinkManager({
  nodes: [{ host: "localhost", port: 2333, authorization: "youshallnotpass" }]
});
```

**Serenity (Rust)**
```rust
use lavalink_rs::LavalinkClient;
let lava_client = LavalinkClient::builder("bot_id")
    .set_password("youshallnotpass")
    .set_host("127.0.0.1")
    .set_port(2333)
    .build();
```

## REST API

KizunaLink exposes a REST + WebSocket API compatible with Lavalink v4. All
`/v4` routes require an `Authorization: <password>` header matching
`server.authorization`.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v4/loadtracks?identifier=` | Load tracks by identifier (URL or `prefix:query`) |
| GET | `/v4/loadsearch?identifier=` | Search tracks (same prefixes as loadtracks) |
| GET | `/v4/decodetrack?encodedTrack=` | Decode an encoded track |
| POST | `/v4/decodetracks` | Decode multiple tracks |
| GET | `/v4/info` | Server info & version |
| GET | `/v4/stats` | Server statistics |
| GET | `/v4/sessions/{id}/players` | List players in a session |
| GET | `/v4/sessions/{id}/players/{guild}` | Get a player |
| PATCH | `/v4/sessions/{id}/players/{guild}` | Update a player (play/pause/volume/filters) |
| DELETE | `/v4/sessions/{id}/players/{guild}` | Destroy a player |
| GET | `/v4/sessions/{id}` | Get session state |
| PATCH | `/v4/sessions/{id}` | Update session (resuming settings) |
| GET | `/v4/lyrics?trackName=&artistName=&query=` | Get lyrics for a loaded track |
| POST | `/v4/sessions/{id}/players/{guild}/lyrics/subscribe` | Subscribe a player to lyric events |
| DELETE | `/v4/sessions/{id}/players/{guild}/lyrics/subscribe` | Unsubscribe from lyric events |
| GET | `/v4/sessions/{id}/players/{guild}/track/lyrics` | Get lyrics for the current/tracked player track |
| GET | `/v4/routeplanner/status` | Route planner status (current IP, rotations) |
| POST | `/v4/routeplanner/free/address` | Free a blocked IP address |
| POST | `/v4/routeplanner/free/all` | Free all blocked IP addresses |
| WS | `/v4/websocket` | WebSocket connection (Lavalink v4 protocol) |
| GET | `/version` | Server version string |
| GET | `/health` | Health probe for orchestrators (no auth required) |
| GET | `/youtube` | YouTube source info (account state) |
| GET | `/youtube/stream/{videoId}` | Resolve a YouTube stream |
| GET | `/youtube/oauth/{refreshToken}` | Refresh a YouTube OAuth token |
| GET | `{metrics.endpoint}` | Prometheus metrics (default `/metrics`, auth required) |

**Search prefixes** (`/v4/loadsearch` and `/v4/loadtracks?identifier=`):

| Prefix | Source |
|--------|--------|
| `ytsearch:` / `ytmsearch:` | YouTube / YouTube Music |
| `spsearch:` / `sprec:` | Spotify search / recommendation |
| `scsearch:` | SoundCloud |
| `jssearch:` | JioSaavn |
| `gnsearch:` | Gaana |
| `dzsearch:` | Deezer |
| `audiussearch:` | Audius |
| `amsearch:` | Apple Music |
| `bcsearch:` | Bandcamp (disabled by default) |
| `shsearch:` | Shazam (disabled by default) |

## TLS (HTTPS/WSS)

KizunaLink can terminate TLS in-process with rustls, so `authorization` is never
sent in cleartext — no reverse proxy required.

```toml
[server.tls]
enabled = true
cert_path = "/etc/kizuna/certs/fullchain.pem"
key_path = "/etc/kizuna/certs/privkey.pem"
```

When enabled, both the REST API (`https://…`) and the WebSocket endpoint
(`wss://…/v4/websocket`) are served over TLS. Leave it disabled if you terminate
TLS at a reverse proxy (Caddy/nginx) instead. `server.tls.enabled = true`
requires both `cert_path` and `key_path`; the server fails fast if the PEM files
are missing or unparseable.

## Environment Variable Overrides

For 12-factor deployments (Docker secrets, Kubernetes Secrets), every config
value can be overridden with a `KIZUNA_*` environment variable. Env vars win
over the TOML file and are applied before startup validation, so a bad value
fails fast:

| Variable | Overrides |
|----------|-----------|
| `KIZUNA_ADDRESS` | `server.address` |
| `KIZUNA_PORT` | `server.port` |
| `KIZUNA_AUTHORIZATION` | `server.authorization` |
| `KIZUNA_RATE_LIMIT_PER_MINUTE` | `server.rate_limit_per_minute` |
| `KIZUNA_TLS_ENABLED` | `server.tls.enabled` |
| `KIZUNA_TLS_CERT` | `server.tls.cert_path` |
| `KIZUNA_TLS_KEY` | `server.tls.key_path` |
| `KIZUNA_LOG_LEVEL` | `logging.level` |
| `KIZUNA_METRICS_ENABLED` | `metrics.prometheus.enabled` |

Example — no `config.toml` secrets on disk at all:

```bash
KIZUNA_AUTHORIZATION="$(cat /run/secrets/lava_password)" \
KIZUNA_ADDRESS=0.0.0.0 \
KIZUNA_PORT=2333 \
./target/release/kizuna-server
```

## Prometheus Metrics

Enable raw metrics with:

```toml
[metrics.prometheus]
enabled = true
endpoint = "/metrics"   # default
```

Requests to the metrics endpoint are authenticated (same `Authorization`
header) and include the KizunaLink REST/WS traffic (request count, latency
histogram) plus process/runtime metrics, ready to scrape with Prometheus.

## KizunaLink vs Lavalink

| Feature | KizunaLink | Lavalink |
|---------|-----------|----------|
| Language | Rust | Java/Kotlin |
| Memory Usage | ~20 MB base | ~150 MB base |
| Startup Time | <1s | ~5-10s |
| Audio Filters | 24 | 14 |
| Source Plugins | 27+ built-in | Plugin system |
| DAVE Encryption | ✅ | ❌ |
| Lyrics Providers | 8 built-in | Via plugins |
| Docker Image Size | ~30 MB | ~200 MB |
| Protocol | Lavalink v4 | Lavalink v4 |

## Troubleshooting

**`libopus not found`**
```bash
# Ubuntu/Debian
sudo apt-get install -y libopus-dev

# macOS
brew install opus

# Arch Linux
sudo pacman -S opus
```

**`Connection refused` on port 2333**
- Ensure KizunaLink is running and the port matches your bot config
- Check firewall rules: `sudo ufw allow 2333`

**YouTube playback fails**
- YouTube frequently changes their API; update to the latest KizunaLink version
- Configure OAuth tokens for more reliable access

## License

MIT License — see [LICENSE](LICENSE) for details.
