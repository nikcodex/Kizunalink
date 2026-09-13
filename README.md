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

## Supported Sources

| Source | Search | ISRC | Lyrics | Status |
|--------|--------|------|--------|--------|
| YouTube | ✅ | — | ✅ | Stable |
| Spotify | ✅ | ✅ | — | Stable |
| Deezer | ✅ | ✅ | ✅ | Stable |
| SoundCloud | ✅ | — | — | Stable |
| Apple Music | ✅ | ✅ | — | Stable |
| Tidal | ✅ | ✅ | — | Stable |
| Bandcamp | ✅ | — | — | Stable |
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
| Shazam | ✅ | — | — | Stable |
| Anghami | ✅ | — | — | Stable |
| Qobuz | ✅ | ✅ | — | Stable |
| Amazon Music | ✅ | ✅ | — | Stable |
| Flowery TTS | ✅ | — | — | Stable |
| Google TTS | ✅ | — | — | Stable |

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

KizunaLink exposes a REST + WebSocket API compatible with Lavalink v4:

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/v4/loadtracks` | Load tracks by identifier |
| GET | `/v4/decodetrack` | Decode an encoded track |
| POST | `/v4/decodetracks` | Decode multiple tracks |
| GET | `/v4/info` | Server info & version |
| GET | `/v4/stats` | Server statistics |
| GET/DELETE | `/v4/sessions/{id}/players/{guild}` | Get/delete player |
| PATCH | `/v4/sessions/{id}/players/{guild}` | Update player |
| PATCH | `/v4/sessions/{id}` | Update session |
| GET | `/v4/lyrics` | Get lyrics for a track |
| WS | `/v4/websocket` | WebSocket connection |

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
