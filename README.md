<div align="center">

# KizunaLink

### A High-Performance, Rust-Native Lavalink v4 Audio Node for Discord

**Sub-second startup · ~25–32 MB RAM footprint · Zero-GC audio pipeline · Native Discord DAVE E2EE · 21 Built-in DSP Filters**

[![Build](https://img.shields.io/github/actions/workflow/status/nikcodex/Kizunalink/ci.yml?branch=main&style=flat-square&logo=github-actions&logoColor=white)](https://github.com/nikcodex/Kizunalink/actions)
[![Release](https://img.shields.io/github/v/release/nikcodex/Kizunalink?style=flat-square&logo=github)](https://github.com/nikcodex/Kizunalink/releases)
[![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Discord Protocol](https://img.shields.io/badge/Lavalink-v4%20Compliant-7289da?style=flat-square)](https://github.com/lavalink-devs/Lavalink)

[Quick start](#quick-start) · [Features](#features) · [Why KizunaLink?](#why-kizunalink) · [Architecture](#architecture) · [Bot integration](#bot-integration) · [Configuration](#configuration) · [API](#rest-and-websocket-api)

</div>

---

## Overview

**KizunaLink** is a standalone, ultra-lightweight audio node that implements the full **Lavalink v4 REST and WebSocket protocol**. Existing Lavalink client libraries and Discord music bots connect to it out of the box without changing a single line of client protocol code.

Built entirely in native Rust with **Tokio**, **Axum**, and **Symphonia**, KizunaLink resolves tracks, decodes audio, resamples and filters PCM, mixes playback layers, encodes Opus via `libopus`, and manages encrypted Discord voice transport—all without the JVM, Spring Boot, or third-party `.jar` plugins.

---

## Why KizunaLink?

| Metric / Feature | **KizunaLink** (Rust) | **Standard Lavalink v4** (JVM) |
|---|:---:|:---:|
| **Runtime** | Native Rust (Single binary) | JVM (Java 17/21 + Spring Boot) |
| **Active Memory Footprint** | **~24 – 32 MB RAM** *(verified live)* | **~350 MB – 1 GB+ RAM** |
| **Startup Time** | **< 0.5 – 1.0 second** | **5 – 15+ seconds** |
| **Audio Jitter & Latency** | **Zero-GC** (Deterministic 50 Hz / 20ms ticks) | Stop-the-world GC sweeps can cause audio stutter |
| **Discord DAVE E2EE** | **Native Built-in** (MLS protocol v1) | Requires external proxy or experimental plugins |
| **DSP Audio Filters** | **21 built-in** (static enum dispatch) | Basic built-in; advanced require `lavadsp` plugins |
| **Sources & Resolvers** | **20+ built-in** (YouTube, Spotify, Deezer, etc.) | Moved to separate 3rd-party `.jar` plugins |
| **SponsorBlock** | **Native Built-in** (auto-skip + WebSocket events) | Requires separate JVM plugin |
| **Lyrics Engine** | **Native Built-in** (7+ concurrent providers) | Requires separate JVM plugin |
| **Hardware Requirements** | Runs on micro VPS ($2/mo), Docker, ARM64/Termux | Requires 1 GB+ RAM and JRE runtime |

---

## Architecture

```text
       Discord Bot / Lavalink Client (wavelink, lavalink-client, poru, etc.)
                                │
                                │ REST + WebSocket (/v4)
                                ▼
┌───────────────────────────────────────────────────────────────────────────┐
│                             KizunaLink Node                               │
│                                                                           │
│  [API & Session Management]                                               │
│   • Axum REST routes & WebSocket session coordinator                      │
│   • Resumable sessions with count- & byte-capped event queues             │
│   • Token-bucket rate limiter per IP with automatic idle cleanup          │
│   • Prometheus /metrics & system telemetry                                │
│                                                                           │
│  [Source & Media Layer]                                                   │
│   • 20+ Sources (YouTube Innertube multi-client spoofing, Spotify,        │
│     Deezer Blowfish decryptor, Apple Music, JioSaavn, SoundCloud, etc.)   │
│   • Scored Best-Match mirror resolver (ISRC, title/artist string distance)│
│   • Native SponsorBlock segment fetcher and auto-skipper                  │
│   • Multi-provider concurrent lyrics engine (LRCLib, Genius, YTM, etc.)   │
│                                                                           │
│  [DSP & Audio Engine]                                                     │
│   • Symphonia demuxers (WebM, MP3, AAC, FLAC, Vorbis, Ogg)                │
│   • Linear / Hermite (cubic) / Sinc resamplers to 48,000 Hz stereo        │
│   • 21 statically dispatched DSP filters (zero vtable overhead)           │
│   • Multi-track mixer with exponential soft-knee limiter (soft_clip_i16)  │
│   • Segregated power-of-two pooled buffer allocator (BufferPool)          │
│   • Zero-copy Opus passthrough when no DSP filters are active             │
│                                                                           │
│  [Discord Voice Gateway & Transport]                                      │
│   • Discord Voice Gateway v4/v8 WebSocket client                          │
│   • Strict 50 Hz (20ms) RTP transmission loop with silence padding        │
│   • Modern ciphers: aead_aes256_gcm_rtpsize & aead_xchacha20_poly1305    │
│   • Native DAVE E2EE (MLS protocol) with staged epoch transition support  │
└───────────────────────────────────────────────────────────────────────────┘
```

---

## Features

### Audio Engine & DSP
- **Zero-GC Audio Pipeline**: Uses segregated power-of-two pooled buffers (`BufferPool`) to eliminate heap allocations and fragmentation in the 50 Hz hot loop.
- **Hardware Denormal Protection**: Configures CPU FPU hardware flags (`FTZ`/`DAZ`) before runtime creation to prevent microcode slow-paths during floating-point filter calculations.
- **21 Built-in Audio Filters**: Dispatched with static compile-time enums (`ConcreteFilter`) eliminating dynamic dispatch overhead:
  - **Equalizer**: 15-band biquad peaking EQ (25 Hz to 16 kHz).
  - **Timescale**: WSOLA-based time stretching and pitch scaling.
  - **Karaoke**: Center-channel phase cancellation vocal remover.
  - **Reverb, Echo, Chorus, Flanger, Phaser, Tremolo, Vibrato, Rotation (8D), Distortion, Compressor, Low-Pass, High-Pass, Normalization, Spatial 3D Audio, and Phonograph (vinyl simulator)**.
- **Exponential Soft Limiter**: Built-in soft-knee limiting (`soft_clip_i16` at 90% full scale) smoothly compresses audio peaks, preventing digital square-wave distortion.
- **Micro-ramped Flow Controller**: Smooth volume, pause, and seek ramps eliminate audible audio pops and clicks.

### Voice Transport & Security
- **Native DAVE (End-to-End Encryption)**: Full implementation of Discord's DAVE protocol using Messaging Layer Security (MLS) via patched `davey` crate, supporting epoch transition staging (Opcode 22/23 contract).
- **RTP Packet Encryption**: Native support for modern Discord voice ciphers (`aead_aes256_gcm_rtpsize` and `aead_xchacha20_poly1305_rtpsize`).
- **Resilient Connection Loop**: Automatic UDP discovery, keepalives, session resumption, and silence frame generation (`MAX_SILENCE_FRAMES = 5`) to cleanly close audio streams.

### Comprehensive Media Sources
- **YouTube**: Multi-client Innertube spoofing (TV Cast, Android VR, Android, iOS, Web Remix, Web Embedded), automatic cipher solving / `n-sig` deobfuscation, startup cipher cache warming, and HLS live stream demuxing.
- **Streaming Platforms**: Spotify, Deezer (with track decryptor), Apple Music, SoundCloud, Tidal, JioSaavn, Gaana, Mixcloud, NetEase, VK Music, Yandex Music, Audiomack, Audius, Twitch, Reddit, HTTP URLs, and Local files.
- **Scored Mirror Resolver**: Automatically resolves metadata-only sources (e.g. Spotify, Apple Music) into playable audio streams using ISRC tags, string similarity, and duration tolerances.
- **Native SponsorBlock**: Queries skip segments from the SponsorBlock API, merges contiguous segments, auto-seeks past sponsors, and emits plugin-compatible WebSocket events.
- **Concurrent Lyrics**: Parallel queries across YouTube Music, LRCLib, Genius, Musixmatch, NetEase, Deezer, and Letras.mus with synced (timestamped) and unsynced delivery.

---

## Quick Start

### Option 1: Pre-built Binaries (Fastest)

Download the standalone binary for your architecture from the [Latest Release](https://github.com/nikcodex/Kizunalink/releases):

```bash
# Example for Linux (x86_64)
curl -LO https://github.com/nikcodex/Kizunalink/releases/latest/download/kizunalink-linux-amd64
chmod +x kizunalink-linux-amd64

# Example for Linux / Android (ARM64 / aarch64)
curl -LO https://github.com/nikcodex/Kizunalink/releases/latest/download/kizunalink-linux-arm64
chmod +x kizunalink-linux-arm64

# Copy configuration and run
curl -LO https://raw.githubusercontent.com/nikcodex/Kizunalink/main/config.example.toml
cp config.example.toml config.toml
./kizunalink-linux-amd64
```

### Option 2: Docker Compose

```bash
git clone https://github.com/nikcodex/Kizunalink.git
cd Kizunalink

cp config.example.toml config.toml
# Edit config.toml and customize server.authorization
nano config.toml

docker compose up -d
curl -s http://127.0.0.1:2333/health
```

### Option 3: Build from Source

#### Prerequisites
- Rust stable toolchain (`cargo`, `rustfmt`, `clippy`)
- CMake, pkg-config, and native C compiler
- `libopus` development headers (`sudo apt install -y build-essential cmake pkg-config libopus-dev`)

```bash
git clone https://github.com/nikcodex/Kizunalink.git
cd Kizunalink

cargo build --release
cp config.example.toml config.toml
./target/release/kizuna-server
```

---

## Bot Integration

KizunaLink accepts connections from any Lavalink v4 client library. Use `127.0.0.1:2333` and your configured `server.authorization`.

### Python (`wavelink`)
```python
import discord
from discord.ext import commands
import wavelink

bot = commands.Bot(command_prefix="!", intents=discord.Intents.all())

@bot.event
async def on_ready():
    node = wavelink.Node(
        uri="http://127.0.0.1:2333",
        password="youshallnotpass",
    )
    await wavelink.Pool.connect(nodes=[node], client=bot)
    print(f"Logged in as {bot.user}")

@bot.command()
async def play(ctx, *, search: str):
    if not ctx.guild.voice_client:
        player = await ctx.author.voice.channel.connect(cls=wavelink.Player)
    else:
        player = ctx.guild.voice_client

    tracks = await wavelink.Playable.search(f"ytsearch:{search}")
    if tracks:
        await player.play(tracks[0])
        await ctx.send(f"🎶 Playing: **{tracks[0].title}**")
```

### JavaScript / TypeScript (`lavalink-client`)
```typescript
import { LavalinkManager } from "lavalink-client";

const manager = new LavalinkManager({
  nodes: [
    {
      host: "127.0.0.1",
      port: 2333,
      authorization: "youshallnotpass",
      secure: false,
    },
  ],
  sendToShard: (guildId, payload) => client.guilds.cache.get(guildId)?.shard.send(payload),
  client: { id: client.user.id, username: client.user.username },
});
```

---

## Configuration

KizunaLink loads `config.toml` from the working directory (falling back to `config.example.toml`). Settings can be customized directly or overridden with environment variables.

```toml
[server]
address = "0.0.0.0"
port = 2333
authorization = "replace-with-your-strong-secret"
player_update_interval = 5        # seconds
stats_interval = 30               # seconds
rate_limit_per_minute = 2000      # requests/min per client IP (0 disables)

[player]
buffer_duration_ms = 400
frame_buffer_duration_ms = 5000
resampling_quality = "medium"     # low | medium | high
opus_encoding_quality = 10        # 1-10

[player.sponsorblock]
enabled = true
categories = ["sponsor", "intro", "outro", "interaction", "selfpromo", "music_offtopic"]
```

### Environment Overrides

| Environment Variable | Config Path |
|---|---|
| `KIZUNA_ADDRESS` | `server.address` |
| `KIZUNA_PORT` | `server.port` |
| `KIZUNA_AUTHORIZATION` | `server.authorization` |
| `KIZUNA_RATE_LIMIT_PER_MINUTE` | `server.rate_limit_per_minute` |
| `KIZUNA_LOG_LEVEL` | `logging.level` |
| `KIZUNA_TLS_ENABLED` | `server.tls.enabled` |
| `KIZUNA_METRICS_ENABLED` | `metrics.prometheus.enabled` |

---

## REST and WebSocket API

All `/v4` endpoints require the `Authorization` header. `/health` is unguarded for orchestrators.

| Method | Endpoint | Purpose |
|---|---|---|
| `GET` | `/health` | Unauthenticated liveness probe |
| `GET` | `/v4/info` | Node version, sources, and active filters |
| `GET` | `/v4/stats` | Memory usage, CPU load, and uptime statistics |
| `GET` | `/v4/loadtracks?identifier=` | Search or resolve track URLs |
| `GET` | `/v4/loadsearch?identifier=` | Query search endpoints |
| `GET` | `/v4/decodetrack?encodedTrack=` | Decode single base64 track payload |
| `POST` | `/v4/decodetracks` | Decode array of base64 tracks |
| `GET` | `/v4/sessions/{session}/players` | List all active players |
| `PATCH` | `/v4/sessions/{session}/players/{guild}` | Play, pause, seek, set volume, and configure filters |
| `DELETE` | `/v4/sessions/{session}/players/{guild}` | Destroy a guild player |
| `GET` | `/v4/lyrics?track=` | Query lyrics across active providers |
| `WS` | `/v4/websocket` | Real-time Lavalink v4 event stream |
| `GET` | `/metrics` | Prometheus metrics exporter |

---

## Development

```bash
# Format check
cargo fmt --all -- --check

# Test suite
cargo test --workspace --all-targets

# Linter
cargo clippy --workspace --all-targets -- -D warnings
```

---

## License

Distributed under the [MIT License](./LICENSE).

---

<div align="center">

Made with ❤️ by [nikcodex](https://github.com/nikcodex)

[Back to top](#kizunalink)

</div>
