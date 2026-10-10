# AGENTS.md — KizunaLink

Rust-native Lavalink v4 audio node for Discord. Cargo workspace:
`kizunalink` (lib, under `kizunalink/kizuna-voice/`) + `kizuna-server` (axum binary).

## Build & run

```bash
export LIBOPUS_STATIC=1 OPUS_STATIC=1   # required; libopus is linked statically
cargo build --release --workspace        # release: panic="unwind" (decoder catch_unwind), strip=true; ~5 min cold
cargo check --workspace --all-targets
cargo test --workspace                   # 249 tests (+1 ignored soak)
cargo test -p kizuna-server -- --ignored soak --nocapture   # soak harness
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check advisories
cargo audit                             # authoritative advisory gate (see Security)
```

Build metadata (`BUILD_TIME*`, `GIT_COMMIT*` shown in the banner and `/v4/info`)
is read via `option_env!`; release CI and the Dockerfile inject it. Unset env
vars render as `unknown`/`0`, so local builds are unaffected.

The binary loads `config.toml` from the current working directory. Set
`KIZUNA_CONFIG_PATH` to an absolute path to load a config from elsewhere (useful
when the working directory is read-only or shared). Copy
`config.example.toml` to `config.toml`.

`server.authorization` has **no default** and must be set. Startup fails if it is
missing, empty, or a known placeholder (e.g. `youshallnotpass`), on **any** bind
address including loopback. Set a strong, unique secret — preferably via
`KIZUNA_AUTHORIZATION` rather than the file. Secrets are overridable via
`KIZUNA_*` env vars (list in `kizunalink/kizuna-voice/config/mod.rs`), which is
how `docker-compose.yml` injects them. A reverse proxy is not a substitute for
the secret: bind to a private address and keep the secret set.

## Testing against a running node

```bash
KIZUNA_ADDRESS=127.0.0.1 KIZUNA_PORT=2333 KIZUNA_AUTHORIZATION=test-secret-123 ./target/release/kizuna-server
curl -H "Authorization: test-secret-123" http://127.0.0.1:2333/v4/info
```

`/health` is unauthenticated; `/v4/*` and `/metrics` require the `Authorization`
header (401 = missing, 403 = wrong). The WebSocket handshake requires
`Authorization`; `User-Id` (numeric Discord user id) and `Client-Name` are
optional. A connection without a valid `User-Id` is accepted, but voice updates
are dropped (`opcodes.rs` logs and ignores them), so real clients must send it.
`Session-Id` enables resumption. `python3` + `pip install websockets` works for
protocol tests.

`scripts/e2e_pause_resume.py` is the real-client regression for the
pause/resume-mid-ramp silent-playback bug (see Gotchas). It drives discord.py +
wavelink against a live node, then bursts `pause(); resume()` with a gap short
enough that the resume lands inside the 500 ms tape stop-ramp, and gates on the
player still advancing position afterwards. It needs `DISCORD_TOKEN` and a
second member in the voice channel (DAVE must form, or the RTP loop emits
nothing). Run it after any change to `TapeEffect`, `FlowController` or
`Mixer::mix`:

```bash
DISCORD_TOKEN=... GUILD=<id> VOICE_CHANNEL=<id> KIZUNA_AUTHORIZATION=... \
  python3 scripts/e2e_pause_resume.py
```

Exit 0 = gate passed, 1 = regression, 2 = inconclusive (no DAVE-ready audio
path).

## Layout

- `kizuna-server/src/{main.rs, health.rs, tls.rs, monitoring/, server/, api/{rest,ws}/}`
- `kizunalink/kizuna-voice/{config, engine, media, discord, lavalink, common}/`
- `vendor/davey` — vendored DAVE (E2EE) crate, patched via `[patch.crates-io]`

## Security

- Dependency advisories are checked by BOTH `cargo deny check advisories`
  (`deny.toml`) and `cargo audit` (`.cargo/audit.toml`). cargo-deny silently
  skips advisories that constrain only `affected.functions` with no floor/ceiling
  in `[versions]` (currently RUSTSEC-2026-0209/0211/0124/0330/0331), so
  **`cargo audit` is the authoritative gate** — do not trust deny alone.
- All current ignores are transitive `libcrux-*` advisories inside the DAVE/MLS
  crypto stack (`davey → openmls_rust_crypto → hpke-rs → hpke-rs-libcrux →
  libcrux-*`); the exact libcrux version is pinned upstream, so there is no
  upgrade path. Keep the two ignore lists in sync and update the reachability
  comments when the crypto stack is bumped.

## Gotchas

- The WebSocket client op set is intentionally small (`IncomingMessage` in
  `kizunalink/.../lavalink/protocol/opcodes.rs`): `voiceUpdate`, `play`, `stop`,
  `destroy`, `configureResuming`. Unknown ops fail to deserialize and are logged
  and dropped. When adding or changing a client op, mirror it in REST (`PATCH
  /v4/sessions/{id}`) so both transports stay in sync, and add a `ws/tests.rs`
  case.
- **`IncomingMessage` field names must be `#[serde(rename = "...")]`-ed one by
  one.** The enum is internally tagged (`tag = "op"`), and in that form
  `rename_all = "camelCase"` only renames the *variant* names — it does NOT touch
  field names. Lavalink clients send `guildId`/`sessionId`/`channelId`, so without
  the explicit renames every real client frame fails with `missing field
  'guild_id'` and is silently dropped. The unit tests in `opcodes.rs` send the
  exact camelCase wire shapes to catch this; keep them when editing the enum.
- DAVE E2EE: `dave.can_send_media()` gates the RTP send loop, so a session that
  never reaches MLS readiness emits nothing (the bot appears silent). Discord only
  completes the MLS group when the channel has other members; a bot alone in a
  channel may never receive the proposal/commit that makes the session ready. Do
  not conclude "DAVE is broken" from a lone-bot test — put a second client in the
  channel, or check for `DAVE session (v1) is READY` in the logs.
- **"Player says PLAYING but the channel is silent" after a pause/resume.**
  `TapeEffect` (the varispeed stop/start ramp) must never be pinned at its 0.01
  floor while the state is `Playing`. `Mixer::mix` drives the ramp from the
  *target* state plus the current rate (`tape.rate()`), not from the state
  transition edge: a `resume()` that lands inside the stop-ramp window
  (`tape_stop_duration_ms`, default 500 ms — wavelink's `pause(); resume()` does
  exactly this) used to skip the start ramp, so the stop ramp completed, the
  state rolled `Starting -> Playing`, and the tape stayed at 0.01 emitting
  permanent silence. Regression test:
  `engine::mix::mixer::tests::tape_recovers_when_resume_lands_mid_stop_ramp`.
  When debugging silent playback, probe RMS *after* `FlowController::process_frame`
  (i.e. after `tape.process`) — decode/resample/encode can all be healthy and the
  tape will still zero the frame.
- YouTube (the most common source) supports a per-source `proxy` like every other
  source; it is what lets the node work from datacenter IPs that YouTube blocks
  (HTTP 403 from `googlevideo.com`). See `default_playback_clients()` and
  `YouTubeConfig::proxy`. The proxy-aware client is threaded into every YouTube
  client *and* into `YouTubeOAuth`; do not construct a bare
  `reqwest::Client::new()` there or OAuth token refresh will bypass the proxy and
  still leave from the blocked host.
- `loadtracks` `loadType` is a closed set (`track`/`playlist`/`search`/`empty`/
  `error`); there is NO `loadFailed` load type. Use `empty` for "no match" and
  `error` (with a `LoadError` payload) for a provider/transport failure. Do not
  collapse a network/HTTP/parse failure into `empty` — clients treat `empty` as a
  normal "nothing found" and swallow the real cause. `LoadResult::Error` is for
  load-time failures; `TrackEndReason::LoadFailed` is a separate WS event reason.
- A `TrackEnd` after a mid-stream decoder error must be `finished`, not
  `loadFailed`: audio already played, so clients should advance the queue. Only a
  failure before any frame is emitted is `loadFailed`. Both still emit a preceding
  `TrackException` carrying the cause. See `stop_reason_after_error` in `monitor.rs`.
- Regional/Indian catalogs (and any YouTube-blocked case) are best served by
  region-native sources: `jssearch:` (JioSaavn) and `gnsearch:` (Gaana) stream from
  their own CDNs and are unaffected by the `googlevideo.com` 403. JioSaavn/Gaana do
  not resolve a bare ISRC, so they belong in the mirrors list as `%QUERY%`
  providers, not `%ISRC%` providers.
- Release uses `panic = "unwind"` so `AudioProcessor::run_guarded` (a `catch_unwind`
  wrapper around the decoder loop) can isolate a panic on one bad stream instead of
  aborting the node. Do not set `panic = "abort"` — it silently makes the guard dead
  code.
- Every REST handler extracts path/query/JSON through `api::rest::json::{ApiPath,
  ApiQuery, ApiJson}` rather than axum's built-ins, so any rejection (bad body, bad
  query, bad path param) returns the same `KizunaLinkError` JSON. An unmatched path
  hits `rest_fallback`, which also emits that shape. Keep new routes on these
  extractors to preserve the single error contract.
- `engine/filters/*` clamp their own inputs (timescale, echo, reverb, phaser), so
  extreme API filter values are safe; do not assume validation lives in the handler.
- State collection takes player read locks with non-blocking `try_read`; unrelated
  to the network I/O done under the player write lock in the REST/WS handlers.
- `docker-compose.yml` runs the image `read_only`; the default config logs to
  `./logs/kizunalink.log`, so the compose file mounts a `kizuna-logs` volume there.
