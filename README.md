# ⛩️ KizunaLink (絆) — High-Performance Discord Audio Engine

> Next-Generation Standalone Discord Voice & Audio Streaming Daemon written in **Rust**.
> Built for zero-lag, ultra-low memory footprints (~15MB RAM), and lossless 320kbps audio streaming.

---

## ⚡ Key Highlights
- **🚀 Ultra-Fast Rust Engine**: Powered by `tokio`, `axum`, and `symphonia`.
- **🧠 Zero GC Jitter**: Unlike Java Lavalink or NodeLink, Rust has no garbage collection pauses.
- **🎵 Native 320kbps Passthrough**: Direct CloudFront CDN stream resolution for JioSaavn, YouTube, and Spotify.
- **☁️ Cloud CI/CD Built**: Zero on-device compilation required. GitHub Actions builds your binaries in the cloud.

---

## 📥 Downloading Pre-Built Binaries

1. Go to the **[Actions tab](https://github.com/)** in your GitHub repository.
2. Click on the latest workflow run.
3. Under **Artifacts**, download:
   - `kizunalink-linux-x86_64` (for standard Linux VPS)

---

## 🚀 Running on Your Server

```bash
# Make binary executable
chmod +x kizunalink-linux-x86_64

# Start KizunaLink
./kizunalink-linux-x86_64
```

---

## 🛠️ Configuration (.env)

```env
KIZUNA_HOST=0.0.0.0
KIZUNA_PORT=2333
KIZUNA_PASSWORD=youshallnotpass
```
