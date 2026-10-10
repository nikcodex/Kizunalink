# AGENTS.md — KizunaLink

Rust-native Lavalink v4 audio node for Discord. Cargo workspace:
`kizunalink` (lib, under `kizunalink/kizuna-voice/`) + `kizuna-server` (axum binary).

## Build & run

```bash
export LIBOPUS_STATIC=1 OPUS_STATIC=1   # required; libopus is linked statically
cargo build --release --workspace        # release: panic="unwind" (decoder catch_unwind), strip=true; ~5 min cold
cargo check --workspace --all-targets
cargo test --workspace                   # 241 tests
cargo test -p kizuna-server -- --ignored soak --nocapture   # soak harness
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check advisories
```

The binary loads `config.toml` from the current working directory. Copy
`config.example.toml` to `config.toml`. The example binds `0.0.0.0` and keeps the
default `authorization = "youshallnotpass"`, which config validation rejects when
binding non-loopback — set a real secret or `KIZUNA_AUTHORIZATION`, or bind
`127.0.0.1`. Secrets are overridable via `KIZUNA_*` env vars (list in
`kizunalink/kizuna-voice/config/mod.rs`), which is how `docker-compose.yml` injects them.

## Testing against a running node

```bash
KIZUNA_ADDRESS=127.0.0.1 KIZUNA_PORT=2333 KIZUNA_AUTHORIZATION=test-secret-123 ./target/release/kizuna-server
curl -H "Authorization: test-secret-123" http://127.0.0.1:2333/v4/info
```

`/health` is unauthenticated; `/v4/*` and `/metrics` require the `Authorization`
header (401 = missing, 403 = wrong). WebSocket handshake needs `Authorization`,
`User-Id`, and optionally `Client-Name` / `Session-Id` (resumption). `python3` +
`pip install websockets` works for protocol tests.

## Layout

- `kizuna-server/src/{main.rs, health.rs, tls.rs, monitoring/, server/, api/{rest,ws}/}`
- `kizunalink/kizuna-voice/{config, engine, media, discord, lavalink, common}/`
- `vendor/davey` — vendored DAVE (E2EE) crate, patched via `[patch.crates-io]`

## Gotchas

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
