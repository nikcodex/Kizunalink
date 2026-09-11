<h1 align="center">KizunaLink</h1>

<p align="center">
  High-performance audio node for Discord bots.
</p>

<p align="center">
  <a href="https://github.com/nikcodex/Kizunalink/releases"><img src="https://img.shields.io/github/v/release/nikcodex/Kizunalink?style=for-the-badge&color=orange&logo=github" alt="Release"></a>
  <a href="https://github.com/nikcodex/Kizunalink/actions"><img src="https://img.shields.io/github/actions/workflow/status/nikcodex/Kizunalink/release.yml?style=for-the-badge&logo=githubactions&logoColor=white" alt="Build Status"></a>
  <br>
  <img src="https://img.shields.io/badge/Language-Rust-orange?style=for-the-badge&logo=rust" alt="Language">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey?style=for-the-badge" alt="Platform">
</p>

---

KizunaLink is a standalone audio sending node for Discord bots. It implements the Lavalink v4 protocol with support for 30+ audio sources, 24 audio filters, DAVE encryption, and lyrics support.

## Features

- **30+ Audio Sources** - YouTube, Spotify, Deezer, SoundCloud, Apple Music, and more
- **24 Audio Filters** - Equalizer, Karaoke, Reverb, Chorus, Flanger, Phaser, etc.
- **DAVE Encryption** - Discord Audio/Video End-to-end encryption
- **Lyrics Support** - 8 lyrics providers (YouTube Music, LRCLib, Genius, etc.)
- **Advanced Pipeline** - Multi-track mixer, 3 resampling algorithms, buffer pool
- **Prometheus Monitoring** - Built-in metrics endpoint

## Quick Start

### Docker

```bash
docker pull ghcr.io/nikcodex/kizunalink:latest
docker run -d -p 2333:2333 ghcr.io/nikcodex/kizunalink:latest
```

### From Source

```bash
git clone https://github.com/nikcodex/Kizunalink.git
cd Kizunalink
cargo build --release
./target/release/kizunalink
```

## Configuration

```toml
[server]
address = "0.0.0.0"
port = 2333
authorization = "youshallnotpass"

[sources.youtube]
enabled = true

[sources.spotify]
enabled = true

[filters]
volume = true
equalizer = true
karaoke = true
# ... 24 filters
```

## License

MIT License - see [LICENSE](LICENSE) for details.