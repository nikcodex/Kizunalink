---
title: Introduction
description: What KizunaLink is, how it relates to Lavalink, and how the pieces fit together.
sidebar:
  order: 0
---

**KizunaLink** (絆, *kizuna* — "bond") is a standalone Discord voice and audio streaming server written in Rust. Your bot talks to it over a small REST + WebSocket API; KizunaLink handles everything else — resolving tracks from streaming services, decoding, filtering, Opus encoding, and sending encrypted RTP to Discord's voice servers.

It implements the **Lavalink v4 protocol**, so any existing Lavalink client library can connect to it unchanged.

## How it fits together

```
┌──────────────┐   Discord Gateway    ┌─────────────────┐
│  Your bot    │◄────────────────────►│  Discord API    │
│ (discord.js, │                      └─────────────────┘
│  discord.py) │                               ▲
└──────┬───────┘                               │ voice UDP / RTP
       │  REST + WebSocket                     │ (Opus, encrypted)
       │  :2333  (Lavalink v4)                 │
       ▼                                       │
┌──────────────────────────────────────────────┴──┐
│  KizunaLink                                     │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │ REST/WS  │ │ Players  │ │ kizuna-voice     │ │
│  │ (axum)   │ │ + queue  │ │ gateway·UDP·DAVE │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │ Sources  │ │ DSP      │ │ Opus encoder     │ │
│  │ ×12      │ │ filters  │ │ (libopus static) │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
└─────────────────────────────────────────────────┘
```

1. Your bot receives a `VOICE_SERVER_UPDATE` and `VOICE_STATE_UPDATE` from Discord and forwards the credentials to KizunaLink with a [player update](/api/rest/#update-player).
2. KizunaLink's embedded voice library, [`kizuna-voice`](/internals/kizuna-voice/), opens the voice gateway (v8) and UDP socket itself — your bot never touches audio.
3. You load tracks with [`/v4/loadtracks`](/api/rest/#load-tracks) and play them by PATCHing the player.
4. KizunaLink streams events (`TrackStartEvent`, `TrackEndEvent`, `playerUpdate`, `stats`…) back over the [WebSocket](/api/websocket/).

## What you get out of the box

| Area | Details |
|---|---|
| **Protocol** | Lavalink v4 REST + WebSocket, `?trace=true` error traces, session resuming |
| **Sources** | JioSaavn, YouTube, YouTube Music, Spotify, Apple Music, Deezer, SoundCloud, Bandcamp, Twitch, Vimeo, NicoNico, direct HTTP and local files |
| **Filters** | volume, equalizer, karaoke, timescale, tremolo, vibrato, distortion, rotation, channelMix, lowPass |
| **Extensions** | Server-side queue, skip/previous, loop modes, autoplay, synced lyrics, `/v4/players/all`, `/v4/sessions` |
| **Ops** | Prometheus metrics, `/health`, systemd unit, Docker image, hardened defaults (SSRF guard, rate limits, body limits) |
| **Voice** | Discord voice gateway v8, `aead_aes256_gcm_rtpsize` encryption, DAVE protocol v1 (MLS) |

## What it is *not*

- **Not a bot.** KizunaLink has no Discord token and never connects to the main gateway. Your bot does that and hands voice credentials over.
- **Not plugin-compatible with Lavalink.** Lavalink's JVM plugins (LavaSrc, LavaSearch, …) can't be loaded. Most of what people use them for — Spotify, Apple Music, Deezer, lyrics — is built in instead.
- **Not a Lavalink fork.** It shares the wire protocol only; the engine, voice stack and source resolvers are original Rust code.

## Requirements

- Linux x86_64 or ARM64 (Android/Termux works). Windows and macOS users should [build from source](/getting-started/build-from-source/) or use Docker.
- Outbound HTTPS to the streaming services you enable, and outbound UDP to Discord's voice servers.
- No Java, no Node.js, no external FFmpeg — Opus is statically linked.

## Next steps

- [Quick Start](/getting-started/quick-start/) — install and play a track in five minutes.
- [Migrating from Lavalink](/getting-started/migrating-from-lavalink/) — if you already run Lavalink.
- [Configuration](/configuration/) — tune sources, security and rate limits.
