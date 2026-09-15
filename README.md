<div align="center">

# ⚡ KizunaLink

**The Rust-native, drop-in Lavalink v4 replacement** — a standalone audio node for Discord bots.

Blazing-fast startup (<1s vs Lavalink's ~5-10s). ~20 MB memory (vs ~150 MB). 24 audio filters. DAVE encryption. 31+ sources.

[Jump to Quick Start](#-quick-start) · [Features](#-features) · [Supported Sources](#-supported-sources) · [REST API](#-rest-api) · [Config](#-configuration)

**Docs:** [Credentials guide](./docs/CREDENTIALS.md) · [Production deployment](./docs/PRODUCTION.md) · [Live test runbook](./docs/LIVE_TEST.md) · [Verification report](./docs/verification.md)

</div>

<p align="center">
  <a href="https://github.com/nikcodex/Kizunalink/releases"><img src="https://img.shields.io/github/v/release/nikcodex/Kizunalink?style=for-the-badge&color=orange&logo=github" alt="Release"></a>
  <a href="https://github.com/nikcodex/Kizunalink/actions"><img src="https://img.shields.io/github/actions/workflow/status/nikcodex/Kizunalink/ci.yml?style=for-the-badge&logo=githubactions&logoColor=white" alt="Build Status"></a>
  <br>
  <img src="https://img.shields.io/badge/Language-Rust-orange?style=for-the-badge&logo=rust" alt="Language">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey?style=for-the-badge" alt="Platform">
  <img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="License">
</p>

---

## 📖 What is KizunaLink?

Your bot's **music brain**. KizunaLink is a standalone audio sending node for Discord bots that implements the **Lavalink v4 protocol**, so it drops straight into any existing Lavalink v4 bot — same REST, same WebSocket, same payloads — just orders of magnitude lighter and faster.

Think "Spotify's audio backend" for your Discord bot: your bot talks to KizunaLink over the Lavalink protocol, KizunaLink does the heavy lifting of resolving tracks, streaming audio, applying 24 DSP filters, and shipping Opus frames to Discord.

> **Why Rust?** Because music servers should be resource-light, fast to boot, and crash-proof. Lavalink is a JVM app; KizunaLink is ~20 MB of static Rust that starts in under a second and sips RAM.

## ✨ Features

<table>
<tr><td width="50%">

### 🎛️ Audio Engine
- **24 DSP filters** — volume, equalizer, karaoke, timescale, tremolo, vibrato, rotation, chorus, reverb, echo, phaser, distortion, compressor, spatial, low/high-pass, channel mix, normalization, phonograph…

- **3 resampling algorithms** — linear, hermite, and sinc (highest-quality) with a thread-safe buffer pool
- **Multi-track mixer** — independent layers per player, output normalized & clamped
- **Opus encode** via native `libopus`, decoded with Symphonia
- **DAVE encryption** — Discord's Audio/Video End-to-End encryption (`v1 mls`)

</td><td width="50%">

### 🎶 Sources & Lyrics
- **31 source plugins** — YouTube, Spotify, Deezer, SoundCloud, Apple Music, Tidal, Pandora, JioSaavn, Gaana, Mixcloud, Netease, VK, Yandex, Audiomack, Audius, Reddit, Twitch, Qobuz, Amazon Music, plus HTTP/reddit/local
- **8 lyrics providers** — YouTube Music, LRCLib, Genius, Musixmatch, Netease, Deezer, Yandex, Letras
- **Search prefixes** — `ytsearch:`, `spsearch:`, `scsearch:`, `jssearch:`, `gnsearch:`…

</td></tr>

<tr><td width="50%">

### 🚀 Ops-Ready
- **Prometheus metrics** at `/metrics` (REST/WS traffic + process/runtime)
- **Structured tracing** (`tracing`) with `RUST_LOG` / `logging.level`
- **Health check** `/health` for orchestrators
- **Graceful shutdown** on SIGINT/SIGTERM; **optional TLS (HTTPS/WSS)**
- **`KIZUNA_*` env-var overrides** — 12-factor / Docker-secrets friendly
- **Fail-fast startup** — bad config dies loudly, not silently

</td><td width="50%">

### 🟢 Protocol & Reliability
- **Lavalink v4 protocol** — drop-in replacement, no bot changes
- **Native SponsorBlock** — skips sponsor/intro/outro segments in-process (plugin-compatible WS events + REST routes)
- **Session resumption** with configurable timeout
- **WebSocket** player events / track events / lyrics updates
- **Route planner** — IP rotation & unblocking for rate-limit-heavy sources
- **Per-IP rate limiting** on REST
- **Status**: 195+ unit tests, clippy `-D warnings` in CI, cross-OS builds
- **[Compatibility verification](./docs/verification.md)** — byte-level parity against Lavalink 4.2.2 + lavaplayer 2.2.6, `youtube-source`, and `SponsorBlock-Plugin`

</td></tr>
</table>

## 🧠 Architecture

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
                                      │  │  31 Source  │ │ Audio Engine  │  │
                                      │  │  Plugins    │ │ ┌───────────┐ │  │
                                      │  │  (YouTube,  │ │ │  Decoder  │ │  │
                                      │  │   Spotify,  │ │ │(Symphonia)│ │  │
                                      │  │   Deezer..) │ │ └─────┬─────┘ │  │
                                      │  └────────────┘ │ ┌────▼─────┐ │  │
                                      │                │ │ Resampler │ │  │
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

## 🚀 Quick Start

Spin up a music node in ~30 seconds. The only thing you must change is `authorization` — everything else works out of the box.

### 🐳 Docker (Recommended)

```bash
# 1. Get the config and tailor it (auth + your Spotify keys if you use them)
cp config.example.toml config.toml
$EDITOR config.toml

# 2. Run
docker compose up -d

# 3. Verify
curl -s http://localhost:2333/health
# → {"status":"ok",...}
```

### 🦀 From Source

**Prerequisites:** Rust 1.88+ and `libopus-dev`

```bash
# Ubuntu/Debian
sudo apt-get install -y libopus-dev cmake pkg-config libclang-dev clang
# macOS
brew install opus cmake pkg-config

git clone https://github.com/nikcodex/Kizunalink.git && cd KizunaLink
cargo build --release
./target/release/kizuna-server
```

### 🗝️ First run — connect your bot

Anything that speaks Lavalink **v4** works unchanged — `lavalink-client` (discord.js), `lavalink-rs` (serenity), `lavaplayer` wrappers, etc. Point it at KizunaLink like any Lavalink node:

```js
// discord.js + lavalink-client
const manager = new LavalinkManager({
  nodes: [{
    host: "localhost",
    port: 2333,
    authorization: "youshallnotpass", // ← change to match config.toml
    secure: false,                    // true when server.tls.enabled
  }]
});
```

Then play something:

```
/v4/loadtracks?identifier=ytsearch:never gonna give you up
```

KizunaLink resolves it, streams it, and your bot (already connected via Lavalink) hears **you** — not a robot.

## ⚙️ Configuration

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

### 🧹 SponsorBlock (built-in)

KizunaLink natively skips sponsor / intro / outro / self-promo segments on
YouTube tracks — no JVM plugin needed. Enable it in `config.toml`:

```toml
[player.sponsorblock]
enabled = true
categories = ["sponsor", "intro", "outro", "interaction", "selfpromo", "music_offtopic"]
# api_url = "https://sponsor.ajay.app"
```

Segments are fetched from the [SponsorBlock API](https://sponsor.ajay.app) when
a track starts, skipped mid-playback with seek, and announced over the WebSocket
as `SegmentsLoaded` / `SegmentSkipped` / `ChaptersLoaded` / `ChapterStarted` —
the same event names the official `SponsorBlock-Plugin` emits, so existing bots
work unchanged.

REST (mirrors the plugin API):

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v4/sessions/:session/players/:guild/sponsorblock/categories` | List active categories |
| `PUT` | `/v4/sessions/:session/players/:guild/sponsorblock/categories` | Replace categories |
| `DELETE` | `/v4/sessions/:session/players/:guild/sponsorblock/categories` | Disable SponsorBlock |
| `GET` | `/v4/sessions/:session/players/:guild/sponsorblock/segments` | Cached segments for active track |

On startup (`AppConfig::load`) KizunaLink reads `config.toml` in the working
directory, falling back to `config.example.toml` if absent. It then applies
`KIZUNA_*` environment-var overrides and validates the result, failing fast on
bad values. See [TLS](#tls-httpswss), [Environment Variable
Overrides](#environment-variable-overrides), and [Prometheus
Metrics](#prometheus-metrics) below.

## 🎵 Supported Sources

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

## 🤖 Bot Integration

KizunaLink is compatible with any Lavalink v4 client library:

**discord.js (v14+)**
```js
const { LavalinkManager } = require("lavalink-client");
const manager = new LavalinkManager({
  nodes: [{ host: "localhost", port: 2333, authorization: "youshallnotpass" }]
});
```

> **Runnable example:** [`examples/discord-bot`](./examples/discord-bot) is a
> complete discord.js v14 + `lavalink-client` bot with a `--self-test` mode that
> verifies the whole voice path (join → voice payloads → `trackStart` → advancing
> position → pause/seek/volume) and tells you which link broke if one does.

**Serenity (Rust)**
```rust
use lavalink_rs::LavalinkClient;
let lava_client = LavalinkClient::builder("bot_id")
    .set_password("youshallnotpass")
    .set_host("127.0.0.1")
    .set_port(2333)
    .build();
```

## 🔌 REST API

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
| `ausearch:` / `audsearch:` | Audius |
| `amsearch:` | Apple Music |
| `bcsearch:` | Bandcamp (disabled by default) |
| `shsearch:` | Shazam (disabled by default) |

## 🔒 TLS (HTTPS/WSS)

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

## 🌍 Environment Variable Overrides

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

## 📊 Prometheus Metrics

Enable raw metrics with:

```toml
[metrics.prometheus]
enabled = true
endpoint = "/metrics"   # default
```

Requests to the metrics endpoint are authenticated (same `Authorization`
header) and include the KizunaLink REST/WS traffic (request count, latency
histogram) plus process/runtime metrics, ready to scrape with Prometheus.

## ⚔️ KizunaLink vs Lavalink

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

## 🧯 Troubleshooting

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

## ⚖️ License

MIT License — see [LICENSE](LICENSE) for details.
