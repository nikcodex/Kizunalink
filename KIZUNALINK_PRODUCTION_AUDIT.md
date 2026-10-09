# KizunaLink Production Audit

**Repository:** `nikcodex/Kizunalink` (workspace: `kizunalink` lib + `kizuna-server` bin, vendored `davey`)
**Audit date:** 2026-10-07
**Auditor:** Buffy (static + architecture audit; **cargo toolchain unavailable in this environment — see §13**)
**Method:** full source review of every Rust module, manifests, config, Docker, CI, and comparison against the official Lavalink v4 wire spec at `lavalink.dev`. No source files were modified.

---

## Executive Summary

KizunaLink is an unusually mature, well-documented Rust re-implementation of a Lavalink v4 audio node. The core is not a prototype: it has a real DAVE E2EE implementation on top of the `davey` crate, a real Symphonia decode → resample → DSP → Opus → RTP → DAVE pipeline, constant-time REST/WS auth, an IP token-bucket rate limiter, SSRF and path-traversal guards, TLS fail-fast, a startup authorization guard, graceful shutdown, and a genuinely comprehensive CI matrix (check / fmt / clippy `-D warnings` / test / 3-OS release build / cargo-deny). The README’s claim of “successful playback at least once” is consistent with a pipeline that is structurally sound.

The audit found **one confirmed wire-protocol defect** (the `GET …/players` response shape), **one confirmed SSRF weakness** (redirects/DNS-rebinding not covered by the URL validator), several performance/robustness risks, and a large, expected **integration-test coverage gap** around live sources, Discord voice, DAVE, and long-running playback. These are the things that separate “plays music” from “production-grade”.

No speculative “DAVE is broken” style claims are made. Where a subsystem could not be exercised (live network, real Discord, real browser), it is marked **NOT TESTED**, never “passing”.

### Scores

| Area | Score |
|---|---|
| Core playback pipeline | 8.0 / 10 |
| Lavalink v4 compatibility | 7.0 / 10 |
| Discord Voice transport | 7.5 / 10 |
| DAVE E2EE | 7.0 / 10 |
| Sources / resolvers (code) | 7.0 / 10 |
| Sources (live reliability) | UNVERIFIED |
| Security | 8.0 / 10 |
| Performance / memory | 7.5 / 10 |
| Deployment / Docker / CI | 8.5 / 10 |
| Testing (unit/integration infra) | 7.0 / 10 |
| Testing (end-to-end/voice/live) | 4.0 / 10 |
| **Overall** | **7.5 / 10** |

### Production status

**READY WITH FIXES** — safe to run in production for typical Lavalink-client workloads once the P0/P1 items below are addressed. The DNS/redirect SSRF hole and the `/players` response shape should be fixed before public exposure and before advertising “v4 compliant” to clients that call the players-list endpoint.

---

## Critical Findings

| ID | Severity | Category | Finding | Evidence | Fix |
|---|---|---|---|---|---|
| COMPAT-001 | **HIGH** | Lavalink v4 | `GET /v4/sessions/{id}/players` returns `{"players":[…]}` instead of a bare JSON array | Official docs example is a bare array; KizunaLink returns `Json(Players{players})` | Serialize `Vec<Player>` directly |
| SEC-001 | **HIGH** | SSRF | `validate_public_url` is bypassable via HTTP redirects and DNS rebinding | Client built without a redirect policy; no re-validation per hop | Custom redirect policy re-validating each hop + pin resolved IP |
| PERF-001 | MEDIUM | Async | Blocking `to_socket_addrs()` DNS resolution on Tokio worker threads | `validate_public_url` called from sync `can_handle` inside async `load` | Move resolution to `spawn_blocking` or resolve once and pin |
| ROBUST-001 | MEDIUM | Robustness | `panic = "abort"` + `expect()` in spawned decoder threads: one spawn failure kills the whole process | `std::thread::Builder::…spawn(…).expect(…)` in `http`/`local`/`youtube` tracks | Return an error / propagate `false` instead of `expect` |
| COMPAT-002 | LOW | Lavalink v4 | `/v4/info` reports `version.semver = "1.1.0"` while forcing `version.major = 4` | `get_info` parse + override | Report a semver consistent with the protocol major (e.g. `4.x.y`) |
| COMPAT-003 | LOW | Lavalink v4 | `Version.build` field absent | docs Version Object includes optional `build` | Add `build: Option<String>` |
| PERF-002 | LOW | Audio | `AudioMixer.enabled` is never re-enabled after `Mixer::stop_all()` | `stop_all` sets `enabled=false`; `add_layer` does not reset it | Re-enable on first `add_layer` |
| PERF-003 | LOW | Alloc | Per-track `expect("failed to spawn … decoder thread")` and per-frame copies | see §8 | see §11 |
| TEST-001 | HIGH | Coverage | No automated test reproduces real playback, Discord voice, or DAVE key exchange | §10 | integration harness (P0) |

---

## Confirmed Bugs

The single wire-level defect confirmed in this audit. Everything else reported is classified as a risk, a performance issue, or a coverage gap — see the labeled sections.

### BUG-001 — `GET /v4/sessions/{sessionId}/players` returns the wrong JSON shape

**Severity:** HIGH
**Status:** Confirmed (also recorded as COMPAT-001)

**File:**
```
kizuna-server/src/api/rest/routes/player/get.rs        (get_players)
kizunalink/kizuna-voice/discord/player/state.rs         (struct Players { players: Vec<Player> })
```

**Problem:**
The handler returns `Json(Players { players })`, which serializes to `{ "players": [ … ] }`. Lavalink v4 returns a **bare JSON array** for this endpoint.

**Evidence:**
The official Lavalink v4 REST documentation (“Get Players”) example payload is a top-level array. The repo’s `api/rest/tests.rs` covers `get_player` (single) and session/player lifecycle but never calls `get_players`, so CI cannot catch it.

**Impact:**
A client calling the players-list endpoint receives an object where it expects an array (`players.forEach`/`players.length` breaks or yields `undefined`). Single-player GET/PATCH/DELETE are unaffected.

**Recommended Fix:**
```rust
let players: Vec<Player> = /* as today */;
(StatusCode::OK, Json(players)).into_response()
```
Remove the `Players` wrapper (and its import) so nothing else depends on it.

**Regression Test:**
Create two players, `GET /v4/sessions/{sid}/players`, assert `body.is_array()`, `len == 2`, and each element has a `guildId` field.

---

## Security Findings

### SEC-001 — HTTP source SSRF guard bypassable via redirects / DNS rebinding

**Severity:** HIGH
**Status:** Confirmed gap (runtime exploitability in a given deployment is **POTENTIAL RISK**; the missing redirect/DNS re-validation is confirmed by code)

**Files:**
```
kizunalink/kizuna-voice/media/sources/http/mod.rs   (validate_public_url)
kizunalink/kizuna-voice/engine/source/client.rs     (create_client — no redirect policy)
```

**Problem:**
`validate_public_url` resolves the host and rejects loopback/private/link-local/multicast/unspecified addresses — but only for the initial URL. The client is built with reqwest defaults, which follow up to 10 redirects and re-resolve DNS at connect time, with no re-validation of redirect targets or the connected IP.

**Evidence:**
`create_client` sets timeouts/keepalive/proxy but **no** `.redirect(...)` policy. `validate_public_url` is called once in `can_handle`/`load`, never per hop.

**Impact:**
An attacker-controlled URL that initially resolves to a public IP can later `302` to `http://127.0.0.1/…` or return a private IP (DNS rebinding), reaching cloud metadata (`169.254.169.254`) or internal services.

**Recommended Fix:**
(1) Custom `redirect::Policy` re-running `validate_public_url` on every hop; (2) resolve once and pin the IP via `ClientBuilder::resolve(...)` so validation and connection share the address; (3) optionally hard-block metadata ranges.

**Regression Test:**
A target returning `302 → http://127.0.0.1/` is refused; a host that resolves public-then-private between validation and connect is refused.

### SEC-002 — Empty `authorization` token is accepted

**Severity:** LOW · **Status:** Confirmed gap
**File:** `kizunalink/kizuna-voice/config/mod.rs` (`validate`)
`validate()` rejects the *default* password on a public bind but does not reject an *empty* `authorization`. Add `if self.server.authorization.is_empty() { return Err(...) }`.

### SEC-003 — Blocking DNS resolution on Tokio workers (also PERF-001)

**Severity:** MEDIUM · **Status:** Confirmed
**File:** `kizunalink/kizuna-voice/media/sources/http/mod.rs`
`to_socket_addrs()` is a blocking syscall invoked from the synchronous `can_handle` while iterating sources inside an async future; a slow/hostile lookup stalls a runtime worker (a denial-of-service vector). Move resolution to `spawn_blocking` or resolve once and pin the IP.

### Positive security findings (NO ISSUE FOUND)
REST + WS auth are constant-time (`subtle`) with Lavalink-correct 401/403; the startup guard rejects the default password on a public bind; local-source path traversal is canonicalized and scope-checked; TLS fails fast with no plaintext fallback; request/WS/track/playlist/queue sizes are bounded; per-IP rate limiting is present.

---

## 1. Repository Architecture Audit

### Workspace

```
Cargo.toml (workspace, resolver 2)
├── kizunalink/            lib crate  (kizuna-voice/*)   ← engine, sources, discord, lavalink protocol
│   ├── build.rs           libopus resolution: LIBOPUS_LIB_DIR → pkg-config → vendored CMake
│   └── native/opus/       vendored libopus source (CMake)
├── kizuna-server/         bin crate  (src/*)            ← REST, WS, sessions, monitoring, TLS
└── vendor/davey/          [patch.crates-io] DAVE (MLS) implementation
```

### Module map

| Layer | Crate / path | Responsibility |
|---|---|---|
| HTTP/WS surface | `kizuna-server/src/api/{rest,ws}` | Axum router, auth middleware, rate limit, WS op dispatch |
| Session/state | `kizuna-server/src/server/{app_state,session,voice}.rs` | `AppState`, `Session`, player map, gateway spawn |
| Lavalink protocol | `kizunalink/kizuna-voice/lavalink/protocol/*` | models, tracks, codec (encode/decode), events, info, stats, opcodes |
| Player | `kizunalink/.../discord/player/*` | `PlayerContext`, `start_playback`, `monitor_loop`, lyrics, sponsorblock |
| Voice gateway | `kizunalink/.../discord/gateway/*` | WS handshake, heartbeat, UDP discovery, `speak_loop`, RTP |
| DAVE | `kizunalink/.../discord/crypto/dave.rs` + `vendor/davey` | MLS/E2EE, epochs, transitions, key exchange |
| Engine | `kizunalink/.../engine/*` | demux, decoder loop, resamplers, 21 DSP filters, mixer, buffer pool, opus codec |
| Sources | `kizunalink/.../media/sources/*` | 20+ source plugins + manager/resolver/best-match |
| Config | `kizunalink/.../config/*` | TOML + `KIZUNA_*` env overrides + `validate()` |
| Monitoring | `kizuna-server/src/monitoring/*` | sysinfo stats, Prometheus |
| TLS | `kizuna-server/src/tls.rs` + `common/tls.rs` | rustls acceptor + provider pinning |

### Dependency / data flow

```
Lavalink Client (wavelink / lavalink-client / poru …)
        │  REST (+ Authorization)         │  WS (/v4/websocket, Authorization + User-Id)
        ▼                                 ▼
   axum REST router                  WS session handler
   (check_auth, rate_limit)          (resolve/resume, ping watchdog, op dispatch)
        │                                 │
        └──────────────┬──────────────────┘
                       ▼
              Session (DashMap)  ── PlayerMap: GuildId → Arc<RwLock<PlayerContext>>
                       │
        PATCH/play ──► start_playback()
                       │
        SourceManager.resolve_track(track_info) ──► Source plugin (YouTube/Spotify/http/local/…)
                       │                                   │
                       │  PlayableTrack::start_decoding()  │
                       ▼                                   ▼
              spawn_blocking ──► AudioProcessor::run()  [ Symphonia demux + decode ]
                       │            │
                       │            ▼  PCM i16 stereo @ source rate
                       │        Resampler (linear/hermite/sinc) → 48 kHz stereo
                       │            │
                       │        flume(bounded) AudioFrame ──► Mixer.add_track()
                       ▼
              VoiceEngine { mixer: Mutex<Mixer>, dave: Option<Shared<DaveHandler>> }
                       │
        speak_loop @ 50 Hz ──► Mixer.mix()      (or opus passthrough)
                       │        FilterChain.process()
                       ▼
              Opus Encode (libopus, 48k stereo, 960 samples) ──► RTP header + XChaCha20
                       │
              DaveHandler.encrypt_opus()  ──► UDPVoiceTransport.transmit_opus()
                       │
                       ▼
                  Discord Voice
```

### Verdict
Architecture is coherent and layered with clean seams (`Engine` trait, `ServerContext`/`SessionContext` hooks, `SourcePlugin` trait). **NO ISSUE FOUND** for overall structure. No `audium` / `audium-server` leftovers exist anywhere in the tree (§10).

---

## 2. Lavalink v4 Compatibility Audit

Verified endpoints present in `kizuna-server/src/api/rest/mod.rs`:

`/v4/loadtracks`, `/v4/loadsearch` (extension), `/v4/info`, `/v4/stats`, `/v4/decodetrack`, `/v4/decodetracks`, `/v4/sessions/{id}`, `/v4/sessions/{id}/players`, `/v4/sessions/{id}/players/{guild}` (GET/PATCH/DELETE), `/v4/routeplanner/*`, `/version`, plus lyrics/sponsorblock/youtube extensions and `/v4/websocket`.

Endpoints named in the brief that are **intentionally absent** and correctly so (they are *not* part of core Lavalink v4): `/v4/encode`, `/v4/encodetracks`, `/v4/sessions/{id}/players/{guild}/voice`, `…/filters`, `…/state`, `…/tracks`. Core Lavalink models voice/filters/state as fields of the `Player` object reached through `PATCH …/players/{guild}`; encode endpoints are a plugin. **Classified NO ISSUE FOUND** — absence matches upstream.

### Compatibility table

| Feature | Status | Evidence | Fix |
|---|---|---|---|
| `/v4/loadtracks` | ✅ Correct | `track.rs`; `Empty{}` serializes `"data": null`; error shape matches | — |
| `/v4/loadsearch` | ✅ Extension (OK) | `track.rs` returns `SearchResult`; 204 when no source | — |
| `/v4/decodetrack` | ✅ Correct | GET `?encodedTrack=`/`?track=` | — |
| `/v4/decodetracks` | ✅ Correct | POST array body, capped at 256 tracks / 2 MiB | — |
| `/v4/info` | ⚠️ Minor | `sourceManagers`, `filters`, `plugins`, `git`, `jvm`, `lavaplayer` present; **`semver` 1.x while `major`→4**; no `version.build` | see COMPAT-002/003 |
| `/v4/stats` | ✅ Correct | `frameStats` omitted here (`collect_stats(&state, None)`) as required | — |
| **`/players` (list)** | ❌ **BUG** | returns `{"players":[…]}`; docs require a bare array | COMPAT-001 |
| `/players/{guild}` GET | ✅ Correct | bare `Player` object | — |
| Player update (PATCH) | ✅ Correct | `track`/`encodedTrack`/`identifier`/`position`/`endTime`/`volume`/`paused`/`filters`/`voice`/`noReplace` | — |
| Player destroy (DELETE) | ✅ Correct | `204 No Content`, emits `TrackEnd{cleanup}` | — |
| Filters | ✅ Correct | full v4 filter set + `pluginFilters`; disabled filters rejected 400 | — |
| Voice | ✅ Correct | `VoiceState{token,endpoint,sessionId,channelId?}`; null `channelId` = disconnect (v4 semantics) | — |
| Session PATCH/GET | ✅ Correct | `{resuming,timeout}` | — |
| Session resume | ✅ Correct | `Session-Id` header, `Session-Resumed`, queued events, generation guard | — |
| WebSocket opcodes | ✅ Correct | in: `voiceUpdate`/`play`/`stop`/`destroy`; out: `ready`/`playerUpdate`/`stats`/`event` | — |
| WS auth | ✅ Correct (001 → 401) | constant-time compare | — |
| Error responses | ✅ Correct | `KizunaLinkError{timestamp,status,error,message,path,trace?}` matches Spring shape | — |
| Track codec | ✅ Correct | v1/v2/v3 decode, v3 encode, `userData` tail | — |

### COMPAT-001 — `GET /v4/sessions/{sessionId}/players` returns the wrong JSON shape

**Severity:** HIGH
**Status:** CONFIRMED COMPATIBILITY ISSUE

**File:** `kizuna-server/src/api/rest/routes/player/get.rs` → `get_players()`
and `kizunalink/kizuna-voice/discord/player/state.rs` → `struct Players { players: Vec<Player> }`

**Current behavior:** the handler returns `Json(Players { players })`, which serializes as
```json
{ "players": [ { "guildId": "…", … } ] }
```

**Expected Lavalink v4 behavior:** the official docs (“Get Players”) show a **top-level JSON array**:
```json
[ { "guildId": "…", "track": {…}, "volume": 100, "paused": false, "state": {…}, "voice": {…}, "filters": {…} }, … ]
```
This is also how established clients parse it (`lavalink-client` `getPlayers()` → `Player[]`, wavelink `players` list).

**Why clients can break:** a client that does `const players = await getPlayers(sessionId); players.forEach(...)` or `players.length` receives an object, not an array. Depending on the client this throws (`forEach is not a function`) or silently returns `undefined`. Because it compiles and the existing REST test never calls `/players`, the defect is invisible to CI.

**Evidence:** official Lavalink v4 REST docs example payload (lavalink.dev/api/rest → “Get Players”) is an array; KizunaLink's `Players` wrapper adds a key. The repo’s own `rest/tests.rs` covers `get_player` (single) but **never** `get_players` (list).

**Recommended fix:**
```rust
// get.rs
let players = /* Vec<Player> as today */;
(StatusCode::OK, Json(players)).into_response()
```
Drop or repurpose the `Players` wrapper (and the `to_response`/`Players` import in `state.rs`) so nothing else regresses.

**Regression test:** call `GET /v4/sessions/{sid}/players` after creating 2 players and assert `body.is_array()` and `body.as_array().unwrap().len() == 2`, plus that each element has `guildId`. Add to `kizuna-server/src/api/rest/tests.rs`.

### COMPAT-002 — `/v4/info` semver vs. forced major

**Severity:** LOW · **Status:** CONFIRMED COMPATIBILITY ISSUE
**File:** `kizuna-server/src/api/rest/routes/stats/info.rs` → `get_info()`
`version.semver` is the crate version `1.1.0`, but `version.major` is forced to `4` (correctly, so clients gating on `major >= 4` accept the node). The inconsistency only matters to clients that parse `semver` rather than `major`.
**Fix:** emit a protocol-facing semver, e.g. `4.0.0` (+ optional build/pre-release), or document the divergence. **Test:** assert `version.major >= 4` and that `semver` either starts with `4.` or is explicitly documented.

### COMPAT-003 — missing `version.build`

**Severity:** LOW · **Status:** CONFIRMED COMPATIBILITY ISSUE
**File:** `kizunalink/kizuna-voice/lavalink/protocol/info.rs` → `struct Version`
The v4 Version object includes an optional `build` string. Add `pub build: Option<String>` with `#[serde(skip_serializing_if="Option::is_none")]`.

---

## 3. Search & Source Audit

`SourceManager` (`media/sources/manager/*`) dispatches `can_handle` → first match; supports `load`, `load_search`, `resolve_track`, and a scored mirror resolver (`%ISRC%`/`%QUERY%` with title/artist/duration weights).

Registration is config-gated and defensively guarded (e.g. Deezer disabled unless `arls` + `master_decryption_key`; Yandex disabled without `access_token`; local refused without `media_dir`).

### Source Compatibility Matrix

Legend — **Search/Load/Playback**: `✅` implemented (code present), `⚠️` partial, `❌` absent, `—` n/a. **Tested** = there is an automated test that exercises the *code path* (unit/dispatch). **Live** = verified against the real service during this audit (never claimed unless run). The dispatch tests in `manager/mod.rs` only assert routing, not live resolution.

| Source | Search | Load | Playback | Tested (dispatch/unit) | Live | Reliability assessment |
|---|---|---|---|---|---|---|
| YouTube | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | Multi-client Innertube + cipher + HLS; high complexity, network/most fragile |
| YouTube Music (ytmsearch) | ✅ | ✅ | ✅ | ✅ | NOT TESTED | Same as YouTube |
| SoundCloud | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | token manager + reader |
| Spotify | ✅ | ✅ | ⚠️ metadata-only → mirror | ✅ dispatch | NOT TESTED | search/playlist via API, **playback always delegated to mirrors** |
| Deezer | ✅ | ✅ | ✅ (Blowfish reader) | ⚠️ unit only | NOT TESTED | requires ARL + key; disabled by default |
| Apple Music | ✅ | ✅ | ⚠️ token/media-api → mirror | ⚠️ | NOT TESTED | needs `media_api_token` for playback |
| Tidal | ✅ | ✅ | ⚠️ via HiFi API | ⚠️ | NOT TESTED | disabled by default; needs external HiFi API |
| JioSaavn | ✅ | ✅ | ✅ | ✅ dispatch + parser | NOT TESTED | custom decrypt reader |
| Gaana | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | custom crypto reader |
| VK Music | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | needs token/cookie; disabled by default |
| Mixcloud | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | reader |
| Audiomack | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | manager + utils |
| Audius | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | app_name optional |
| Twitch | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | HLS/live, token discover |
| Amazon Music | ✅ | ✅ | ✅ | ✅ dispatch | NOT TESTED | region/validators/parsers present |
| Netease | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled by default |
| Yandex Music | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled; needs token |
| Qobuz | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled; token/app_id/secret |
| Anghami | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled |
| Pandora | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled; needs csrf token |
| Last.fm | ✅ | ✅ | ❌ (metadata) | ⚠️ | NOT TESTED | search/metadata only → mirror |
| Shazam | ❌ disabled | ❌ | ❌ | ✅ | NOT TESTED | **known-broken** (403 to DC traffic) — correctly disabled by default |
| Bandcamp | ❌ disabled | ❌ | ❌ | ✅ | NOT TESTED | **known-broken** (CSP challenge) — correctly disabled by default |
| Reddit | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | manager |
| Google TTS | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled by default |
| Flowery TTS | ✅ | ✅ | ✅ | ⚠️ | NOT TESTED | disabled by default |
| HTTP | ✅ | ✅ | ✅ | ✅ unit (SSRF) | NOT TESTED | seeks, range checks, prefetcher; see SEC-001 |
| Local | ✅ | ✅ | ✅ | ✅ unit (traversal) | NOT TESTED | scoped to `media_dir`, symlink-safe |

**Honest classification:** every non-disabled source is *Implemented*; the ones with dedicated parser/crypto unit tests are *Implemented + tested*; **none** were *live verified* in this audit. Spotify/Apple Music/Tidal/Last.fm are *Implemented, playback via mirrors*. Shazam/Bandcamp are *correctly disabled (known broken)*. No source is claimed as *Broken* — the disabled ones are gated, not deceptive.

`load_search` implementations exist on the source plugins and route through `SourceManager::load_search`. Whether each returns *playable* tracks (a real audio URL that decodes) cannot be asserted statically and is a **live test gap** (P0, §10).

---

## 4. Audio Pipeline Audit

Because playback has worked, the pipeline is treated as **baseline functional**; this section targets reliability under edge cases.

```
Source → Resolver → Reader → Decoder → PCM → DSP → Opus → RTP → DAVE → Discord
```

| Stage | Implementation | Status | Known issues | Tests |
|---|---|---|---|---|
| Source | `media/sources/*` | Functional | live reliability unverified (§3) | dispatch/parser units |
| Resolver | `manager/{mod,resolver,best_match}.rs` | Functional | **blocking DNS in `can_handle`** (PERF-001) | mirror loop-guard logic present |
| Reader | `engine/source/http/{mod,prefetcher}.rs`, per-source readers | Functional | range/`200` misuse is rejected; good | unit? none live |
| Decoder | `engine/processor.rs::AudioProcessor::run` | Functional | recoverable-error backoff; EOF handled; seek resets resampler | via effects? limited |
| PCM / downmix | `processor.rs` | Functional | channel-layout down/up-mix is heuristic (even→L, odd→R), not standard matrix for >2ch | none |
| Resampler | `engine/resample/*` | Functional | passthrough when 48k; rate-change re-init handled | unit? — |
| DSP | `engine/filters/*` (21) | Functional | static dispatch; `validate_filters` guards disabled filters | per-filter units |
| Mixer | `engine/mix/mixer.rs` | Functional | soft-clip limiter; i32 accumulation; **`AudioMixer.enabled` never re-enabled after `stop_all`** (PERF-002) | units present (soft-clip, layers) |
| Opus | `opus/*` + `engine/codec/opus_encoder.rs` | Functional | 48k stereo, 960 samples, complexity from config | units? — |
| RTP | `discord/gateway/udp_link.rs` | Functional | header/seq/ts persists across reconnects via `PersistentSessionState` | — |
| DAVE | `discord/crypto/dave.rs` | Probably correct, under-tested | see §5 | unit tests for buffering/epochs |
| Discord | `discord/gateway/session/voice.rs` | Functional | 50 Hz `MissedTickBehavior::Skip`; silence padding; keepalive | — |

**Edge cases examined (NO ISSUE FOUND unless noted):**

- **Sample rate / channel / frame size** — encoder is fixed 48 kHz stereo, 960 samples/ch = 20 ms; decoder resamples arbitrary input. Consistent.
- **Buffer under/overflow** — decode→mixer channel is `flume::bounded` (backpressure by design); mixer→PCM uses pooled buffers (`acquire_buffer/release_buffer`); HTTP prefetch has a bounded ring. No unbounded frame queues found in the audio path.
- **Blocking on async workers** — decode runs in `spawn_blocking` + a dedicated named OS thread; `speak_loop` uses `try_lock_yield!` to avoid holding locks across awaits. **Exception:** DNS in `validate_public_url` (PERF-001).
- **Task leaks / shutdown** — `Session::register_task` stores abort handles and prunes finished ones; `shutdown()` aborts gateway+track tasks; `main.rs` drains all sessions on SIGTERM. Good.
- **Position/timestamp** — position tracked in samples in the mixer/handle and converted to ms with `OPUS_SAMPLE_RATE`; `endTime` enforced in `monitor_loop`. Consistent.

**PERF-001 (blocking DNS):**
**File:** `media/sources/http/mod.rs::validate_public_url` (called by sync `can_handle` and by `load`).
`std::net::ToSocketAddrs::to_socket_addrs()` performs a **blocking** syscall; `can_handle` is invoked synchronously while iterating sources inside the async `load`/`resolve_track` futures, so a slow/hostile DNS lookup stalls a Tokio worker.
**Fix:** resolve inside `tokio::task::spawn_blocking`, or resolve once and pin the connected IP (also closes SEC-001's TOCTOU). **Test:** a source whose host resolves slowly must not block other concurrent requests.

**PERF-002 (`AudioMixer.enabled` sticky off):**
**File:** `engine/mix/mixer.rs` — `Mixer::stop_all()` sets `self.audio_mixer.enabled = false`; `AudioMixer::add_layer` never restores it. Any sound-effect layer added after a `stop_all()` (e.g. after `PATCH encodedTrack:null`) is mixed but then discarded by the `enabled` early-return.
**Fix:** set `enabled = true` in `add_layer`. **Test:** add a layer after `stop_all` and assert it is audible.

**ROBUST-001 (`panic=abort` + `expect`):**
**Files:** `media/sources/http/mod.rs`, `local/mod.rs`, `youtube/reader.rs` (`std::thread::Builder::…spawn(…).expect("failed to spawn … decoder thread")`).
With `[profile.release] panic = "abort"`, a single failed thread spawn (resource exhaustion under many players) aborts the entire process rather than failing one track.
**Fix:** replace `expect` with an error send to `err_tx` (the channel already exists for exactly this). **Test:** exercise the failure branch (e.g. injected spawn failure).

---

## 5. DAVE E2EE / Discord Voice Audit

**Implementation:** `kizunalink/kizuna-voice/discord/crypto/dave.rs` over the vendored `davey` (MLS) crate; wire handling in `discord/gateway/session/{handler,voice}.rs`.

**Verdict:** **Probably correct, under-tested (live).**

Positives verified by code + unit tests:

- **Session setup / epochs** — `setup_session` (v1 only, rejects unknown versions), `prepare_epoch` correctly distinguishes `epoch==1` (create group → send key package, opcode 26) from `epoch>1` (retain group → announce readiness, opcode 23), with unit tests `epoch_one_creates_a_group_while_higher_epochs_only_report_readiness`.
- **Transitions** — `prepare_transition`/`execute_transition`, pending map, version only changes on execute; unit-tested (`protocol_version_changes_only_when_transition_executes`).
- **Buffering** — proposals/handshake arriving before the external sender are buffered and replayed; oversized control payloads rejected *before* buffering (unit-tested).
- **Encrypt** — silence frames and pre-transition traffic pass through unencrypted; encryption engages only when `session.is_ready()`; readiness transitions logged.
- **Reconnect/resume** — RTP state persisted via `PersistentSessionState`; `reset()` returns to plaintext on version 0.

**What was NOT verified:** actual MLS key exchange against a real Discord voice channel; DAVE with real *other* users present (user add/remove → commit/welcome); privacy-code round-trip; behavior across a mid-call protocol-version upgrade from the real gateway. These are **NOT TESTED — live Discord dependency unavailable**.

**Recommendations (integration tests, P1):**
1. Table-driven test over `DaveHandler` opcode 22/23/24/26/27/28 sequences captured from a real session (fixtures), asserting state transitions and emitted opcodes.
2. A two-party harness using `davey` directly: A creates group + key package, B processes welcome, both derive matching keys, A encrypts / B decrypts a 20 ms Opus frame.
3. Regression for epoch>1 readiness (the code comment notes this previously stalled the transition — keep it locked).

---

## 6. Player / Session State Machine Audit

`PlayerContext` (`discord/player/context.rs`) + `Session` (`server/session.rs`) + WS/REST update paths.

Lifecycle paths reviewed: create (`get_or_create_player`), connect (`voice_update` → `connect_voice`), play (`start_playback`), pause/resume (`set_paused`), seek (`seek`), stop (`stop_track`/`stop_player`), destroy (`destroy_player` → `destroy`), reconnect/voice-update-while-playing, resume, timeout.

**Findings:**

- **NO ISSUE FOUND** for the race that typically bites: `handle_voice_update` decides *whether to spawn* a gateway task under a single write lock and re-checks after `connect_voice` returns, dropping a stale task if a newer one won. Correct.
- **NO ISSUE FOUND** for duplicate players: `DashMap::entry(...).or_insert_with` is atomic per key.
- **NO ISSUE FOUND** for stale track state: `monitor.rs::clear_player_state` only clears if the handle is the same allocation (`is_same`), avoiding clobbering a replacement track.
- **POTENTIAL RISK (low):** `start_playback` allocates a **new** `stop_signal` Arc each track while the old `MonitorCtx` still holds the old Arc; the old task is aborted first, so this is safe today but is fragile — a future change that lets the old task outlive the swap would miss the stop signal.
- **POTENTIAL RISK (low):** holding the player **write lock** across `connect_voice().await` in `update.rs::handle_voice` means a concurrent `GET /players` read blocks until the (fast) gateway spawn completes. Bound today by connect cost; note as a latency coupling.
- **Test scenarios required** (not currently automated): play→pause→resume, play→seek→resume, play→stop→play, destroy→recreate, disconnect→reconnect, voice-update-while-playing, multi-guild isolation. See §10 P1.

---

## 7. Concurrency Audit

| Mechanism | Locations | Assessment |
|---|---|---|
| `parking_lot::Mutex` | `AppState.{system_state,last_system_refresh,process_stat}`, `Session.{event_queue,task_handles}`, HTTP shared state | short critical sections, no awaits held |
| `tokio::sync::{Mutex,RwLock}` | `PlayerContext` (via `DashMap` value), `filter_chain`, `engine`, `mixer`, DAVE handler | ordered player→engine→mixer; no inverse acquisition found |
| `Arc` / `DashMap` | sessions, resumable sessions, players, rate buckets | ref-counted, no cycles observed |
| channels | flume (audio frames, WS sink), tokio mpsc (WS writer, events) | audio + events bounded; WS writer bounded (1024) |
| `spawn`/`spawn_blocking` | decode (dedicated OS thread), HTTP prefetch, monitor, stats, cleanup | abort handles tracked |
| `select!` | `main.rs` shutdown, WS handler loop | no busy-loop |
| atomics | volume/position/state/ping, frame counters, resume generation | AcqRel/SeqCst where needed |

**Findings:**

- **NO deadlock found.** Lock ordering is consistent (player → engine → mixer) and the audio send loop deliberately acquires locks with `try_lock` + `yield_now` rather than blocking.
- **NO unbounded channel in the audio/event path** except per-track decoder command channels (`flume::unbounded` for `DecoderCommand`), which are command-only (bounded by caller discipline). Low risk; note as **POTENTIAL RISK** (INFO).
- **POTENTIAL RISK:** `WsSession` event bridge uses `tokio::mpsc::channel(256)`; when full, DAVE/voice events are dropped silently by the `while let` consumer? Actually the sender is `try_send`-less: `connect_voice` passes `event_tx` and the consumer forwards with `send_message` (which itself drops on full). Voice events can be dropped under a stalled WS consumer — acceptable backpressure, but observability is limited.

No lock held across an `await` in a way that can deadlock was found.

---

## 8. Memory / Performance Audit

**Design intent:** low memory, zero-GC, deterministic 50 Hz. The design supports this.

- **Allocations in the audio loop:** pooled PCM buffers (`engine/buffer/{pool,ring}.rs` acquired/released per frame); mixer reuses `mix_buf`/`final_pcm_buf`/`acc_buf` (resized, not reallocated per tick); `downmix_buf` cleared/reserved. Good.
- **Cloning:** `track_info.clone()` on state snapshots and event payloads; `Player` snapshots clone voice/filters/track. Acceptable; not per-20ms.
- **Decoder allocations:** Symphonia `SampleBuffer` reused across packets; resampler output buffer acquired from pool with `ceil(len*ratio)+32` headroom. Good.
- **Task cleanup:** session task handles pruned; finished monitor tasks aborted on stop. Good.
- **Potential leak / growth:** `Session.event_queue` is **count- and byte-capped** (`max_queue_size` ≤ 10 000, byte budget 64 KiB–64 MiB). Rate-limiter buckets are cleaned every 5 min (10 min idle TTL). No unbounded growth found.
- **Queue sizes:** WS output 1024, session output channel 1024, event bridge 256, decoder frame channel = `buffer_duration_ms/20`. All bounded.

**No benchmark numbers are invented.** No criterion/bench harness exists. `soak_tests.rs` (ignored by default) creates 8 sessions × 16 players and hammers REST, asserting no leak — the only load harness.

**Recommended benchmarks (P2):** criterion micro-benches for `Mixer::mix`, `AudioProcessor::run` (per-MP3-minute), `OpusCodecEncoder::encode`, and `DaveHandler::encrypt_opus`; plus a soak profile at 1/10/50/100 players recording RSS, CPU, and `frameStats.deficit`.

---

## 9. Security Audit

### Authentication

- **REST:** `middleware.rs::check_auth` — constant-time (`subtle::ConstantTimeEq`), **401** missing / **403** wrong, matching Lavalink. ✅
- **WebSocket:** `ws/mod.rs::websocket_handler` — constant-time, correct header handling, upgraded only after auth. ✅
- **Default password:** `config.example.toml` ships `address="0.0.0.0"` + `authorization="youshallnotpass"`. **A startup guard exists and refuses this combination:**
  ```rust
  // config/mod.rs::validate()
  if self.server.authorization == "youshallnotpass"
      && self.server.address.parse::<IpAddr>().map(|ip| !ip.is_loopback()).unwrap_or(true)
  { return Err("server.authorization must be changed from the default when binding publicly") }
  ```
  ✅ **The stronger authorization guard that reportedly existed in `kizunalink-test` is present in KizunaLink today.** (Loopback-only binding with the default merely warns.) Empty password is not separately rejected — recommend also refusing empty `authorization`.
- **Env overrides:** `KIZUNA_AUTHORIZATION` etc. applied after TOML, malformed values fail fast. ✅ Never logged.

### SSRF (HTTP source)

`media/sources/http/mod.rs::validate_public_url` rejects loopback/private/link-local/multicast/unspecified IPv4 and loopback/ULA/link-local/unspecified IPv6, and rejects non-http(s) schemes — **for the initial URL only**. Unit tests cover `127.0.0.1`, `[::1]`, `file://`, `ftp://`.

**SEC-001 (H1) — redirect & DNS-rebinding bypass:**
The client is built in `engine/source/client.rs::create_client` **without** `.redirect(Policy::none())` or a custom policy, so reqwest follows up to 10 redirects and resolves DNS again at connect time. Neither the redirect targets nor the connect-time IP are re-validated. An attacker who can serve `https://evil.example/audio.mp3` (initial DNS → public IP) can subsequently return a private IP (rebinding) or issue `302 Location: http://169.254.169.254/…` / `http://127.0.0.1:…`, reaching cloud metadata or local services.
**Fix:** (a) build the client with a custom `redirect::Policy` that re-runs `validate_public_url` on every hop; (b) resolve once, pin the IP via `ClientBuilder::resolve(host, addr)` / a `dns` resolver, so validation and connection use the same address; (c) optionally block `169.254.169.254`/metadata ranges explicitly.
**Regression test:** an HTTP target returning `302 → http://127.0.0.1/` must be refused; a host that resolves public-then-private must be refused.

### Path traversal (local source)

`media/sources/local/mod.rs::allowed_path` canonicalizes the candidate and requires `candidate.starts_with(root)` where `root` is the canonicalized `media_dir`; registration refuses the source without `media_dir`. This is correct and symlink-aware. **NO ISSUE FOUND.** (Note: `probe_file`/`open` re-open by path after the check — a filesystem race is theoretically possible but requires local write access to the media dir; low.)

### Resource exhaustion

Request body limit 4 MiB; WS message/frame cap 1 MiB; `loadtracks` identifier cap 16 KiB; `loadsearch` 4 KiB; `decodetracks` ≤ 256 tracks / 2 MiB; paused event queue count+byte capped; per-IP token bucket (default 2000/min) on REST + WS upgrade **and** a per-socket WS message throttle. All bounded. ✅

### TLS

`tls.rs::load_acceptor` fails fast on missing/garbage cert/key (tested), no silent plaintext fallback; provider pinned (`common/tls.rs`). ✅

### Configuration

`validate()` rejects zero port/intervals, out-of-range opus quality, oversized event queue, TLS without cert/key, and the public-default-password case. ✅

---

## 10. Docker / Production Deployment Audit

**Dockerfile (4 stages):**
- `builder` (`rust:1.93-bookworm`) installs cmake/pkg-config/clang/perl/git and runs `cargo build --release --locked`.
- `runtime-base` (`debian:bookworm-slim`): non-root `kizunalink` user, `WORKDIR /app`, `EXPOSE 2333`, `ENTRYPOINT ["/app/kizunalink"]`.
- `ci` (default for release pipeline): copies `bin/linux/${TARGETARCH}/kizunalink`.
- `local` (default target): copies `/build/target/release/kizuna-server` → `/app/kizunalink`.

**Naming consistency — verified, not assumed:**

| Name | Location | Verdict |
|---|---|---|
| package `kizunalink` | `kizunalink/Cargo.toml` | lib crate |
| package `kizuna-server` | `kizuna-server/Cargo.toml` | **produces the binary** `target/release/kizuna-server` |
| `/app/kizunalink` | Dockerfile ENTRYPOINT + both COPY stages | ✅ binary copied **is** `kizuna-server`, renamed to `kizunalink` — **consistent** |
| `kizunalink-linux-{amd64,arm64}` | `release.yml` rename | ✅ matches `bin/linux/<arch>/kizunalink` the `ci` stage expects |
| `audium`, `audium-server` | — | **do not exist anywhere** (grep across `*.rs/*.toml/*.yml/*.md/Dockerfile` = 0 hits) |

So the “KizunaLink / kizuna-server / audium” inconsistency in the brief is **not present in this repository**: the crate-vs-artifact rename is intentional and consistently applied through Dockerfile + release workflow. **NO ISSUE FOUND.** (The package name `kizuna-server` producing an artifact called `kizunalink` is mildly confusing but harmless.)

**docker-compose:** mounts `config.toml:ro`, `cap_drop: ALL`, `no-new-privileges`, `read_only: true`, `tmpfs: /tmp`, `restart: unless-stopped`. ✅ Note: the image is `read_only`, but `config.example.toml` configures a file log at `./logs/kizunalink.log`; with no writable volume that log write fails (non-fatal) unless `logs/` is tmpfs-mounted. Minor deploy note.

**No `HEALTHCHECK` directive** in the Dockerfile, though `/health` exists and is unauthenticated. Recommend adding `HEALTHCHECK CMD` hitting `/health`.

**Graceful shutdown:** SIGINT/SIGTERM handler; drains in-flight HTTP, then explicitly shuts down all active + resumable sessions, aborts gateway/track tasks. ✅

**Pterodactyl/panel:** not present in-repo (no egg/startup files). Not assessed.

---

## 11. CI / Testing Audit

**`.github/workflows/ci.yml`** runs, on push/PR to `main`:
- `cargo check --workspace --all-targets` (with `pipefail`)
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `cargo build --release` on **ubuntu / macos / windows**
- `cargo deny check advisories` (RustSec gate, documented ignores)

**`release.yml`:** builds linux x64/arm64, macOS x64/arm64, Windows x64/arm64, Termux ARM64; renames `kizuna-server` → platform artifacts; docker buildx multi-arch push to GHCR using the `ci` stage; attaches assets to the release. Coherent.

**What CI does NOT test** (all confirmed absent from the test tree):
- Real YouTube/Spotify/etc. search or resolution
- Real track playback (decode → Opus → output)
- Real Discord voice connect / UDP discovery / RTP
- DAVE key exchange with a real or simulated second party
- Multiple simultaneous *playing* players (only REST churn at 16 players/session)
- Network failures / source failures / reconnect / session resume under load
- Long-running playback (> minutes) and memory growth over time

**CI passing ≠ production-ready** — the unit/integration suite is strong for protocol/parsing/state, weak for media/voice.

---

## 12. Test the Unique Improvements From `kizunalink-test`

**The `kizunalink-test` repository is not present in this workspace**, so it could not be diffed directly. Auditing the current tree against the specific improvements the brief attributes to it:

| Reported improvement | Exists in KizunaLink? | Action |
|---|---|---|
| Startup authorization protection | **Yes** | `config/mod.rs::validate()` rejects default password on public bind (see §9). Nothing to port. |
| `load_search` implementations | **Yes** | `SourceManager::load_search` + per-source `load_search` + `/v4/loadsearch`. Nothing to port. |
| Response-shape fixes | **Partial** | Error/empty/exception shapes correct; **`/players` shape is wrong** (COMPAT-001). Fix belongs in KizunaLink, not ported from elsewhere. |
| Session/player endpoint behavior | **Mostly yes** | PATCH/GET/DELETE + session PATCH/GET correct; `/players` list bug remains. |
| Lavalink v4 player-listing fix | **No / uncertain** | If `kizunalink-test` contains the bare-array `get_players` fix, **that is the one change worth porting** — it matches COMPAT-001. |
| Regression tests for the above | **Partial** | REST tests cover info/stats/session/player lifecycle but **not** the `/players` list. |

**Recommendation:** do **not** delete `kizunalink-test` until its `get_players` implementation and any `/players` regression test are compared against COMPAT-001. Everything else listed is already present in KizunaLink.

---

## 13. Actual Runtime Verification

**First pass: toolchain unavailable. Now partially re-verified — see “Final Verification Pass” at the end of this document.** A Rust toolchain (1.99.0) was subsequently installed and `cargo check --workspace --all-targets` **passed (exit 0)**; `cargo fmt --check` **failed on 1 file**. The text below is the original first-pass status and is superseded by the final pass.

```
$ cargo --version   → (first pass: command not found; final pass: 1.99.0 installed)
$ rustc --version   → (first pass: command not found; final pass: 1.99.0 installed)
$ cmake --version   → cmake: command not found (still absent — not required for `cargo check`)
$ pkg-config --modversion opus → pkg-config: command not found (still absent)
$ which rustup → (first pass: none; final pass: installed)
```

Consequently the following could **not** be executed here and must be run in a toolchain-equipped environment (the repo’s own CI does exactly this):

| Command | Result here |
|---|---|
| `cargo fmt --all -- --check` | NOT RUN — no toolchain |
| `cargo check --workspace --all-targets` | NOT RUN — no toolchain |
| `cargo clippy --workspace --all-targets -- -D warnings` | NOT RUN — no toolchain |
| `cargo test --workspace --all-targets` | NOT RUN — no toolchain |
| `cargo build --release` | NOT RUN — no toolchain (also needs cmake/pkg-config for vendored opus) |
| Live source resolution | NOT TESTED — external dependency unavailable |
| Discord voice / DAVE | NOT TESTED — external dependency unavailable |

**No claim of a passing build/test is made.** Reviewers should treat the compile status as *presumed good* (the code is written against stable Rust 2024, `rust-version = "1.88"`, and CI enforces fmt/clippy/test) but **unverified by this audit**.

---

## Performance

- **Allocation hotspots:** per-packet `SampleBuffer` (reused), resampler output buffers (pooled), `Vec<Player>` snapshots on `GET /players` (clones every player), `serde_json` serialization of events. None per-20 ms except pooled buffers.
- **CPU hotspots:** Opus encode (libopus), resampling (sinc at `high`), filter chain, DAVE encryption (MLS/ChaCha), and `System`/`ProcessStat` refresh (throttled to 5 s).
- **Memory risks:** low; the only sizeable bounded buffers are the HTTP prefetch ring and the paused event queue (capped).
- **Concurrency risks:** lock coupling `player → engine → mixer`; `connect_voice` awaited under the player write lock (latency, not deadlock).
- **Queue sizes:** decoder frame = `buffer_duration_ms/20`; WS out 1024; session out 1024; event bridge 256; paused queue ≤ 10 000 items / ≤ 64 MiB.
- **Potential leaks:** none found; session/task/rate-bucket cleanup is present.
- **Recommended benchmarks:** see §8.

**No benchmark numbers are fabricated.**

---

## Production Deployment

| Item | Assessment |
|---|---|
| Docker | Multi-stage, non-root, `ci`/`local` targets consistent with release workflow ✅ |
| Binary | `kizuna-server` → `/app/kizunalink`; ENTRYPOINT matches ✅ |
| Pterodactyl | Not present in-repo — N/A/Not assessed |
| Configuration | TOML + `KIZUNA_*` overrides + fail-fast `validate()` + public-default-password guard ✅ |
| TLS | rustls acceptor, fail-fast, provider pinned ✅ |
| Authentication | constant-time REST/WS, 401/403 semantics ✅ |
| Logging | `tracing` + file rotation; **file log path unwritable on `read_only` image** ⚠️ |
| Metrics | Prometheus behind auth + rate limit; latency histogram ✅ |
| Graceful shutdown | SIGTERM drain of HTTP + sessions + tasks ✅ |
| Healthcheck | endpoint exists; **no Dockerfile `HEALTHCHECK`** ⚠️ |

---

## Test Coverage Gaps

Prioritized tests that should exist but currently do not:

**P0**
1. `GET /v4/sessions/{id}/players` asserts a **bare array** (COMPAT-001).
2. End-to-end playback harness against a local fixture (file source): decode → mixer → opus → assert non-silent frames and correct position/endTime.
3. HTTP SSRF regression: redirect-to-private and rebind scenarios refused (SEC-001).

**P1**
4. Discord voice handshake + UDP discovery + RTP transmit against a mock voice server.
5. DAVE two-party MLS key exchange (create/welcome/commit → matching keys → encrypt/decrypt a frame).
6. Player state-machine matrix: play→pause→resume, play→seek→resume, play→stop→play, destroy→recreate, disconnect→reconnect, voice-update-while-playing.
7. Multi-player isolation: N guilds playing concurrently; no cross-talk, correct per-session `frameStats`.
8. Session resume under load: queued events delivered in order after reconnect; timeout expiry tears down cleanly.

**P2**
9. Source live-smoke tests (opt-in, network-gated) for each enabled source: search returns ≥1 item and item resolves to a decodable stream.
10. Long-run soak (hours) asserting flat RSS and `frameStats.deficit ≈ 0`.
11. Failure injection: decoder error mid-stream → `TrackException`; stuck track → `TrackStuck`; source timeout → `loadFailed`.

**P3**
12. Fuzz `decode_track` on arbitrary base64 (panic-free), `validate_public_url`, filter JSON.
13. Windows/macOS CI smoke of the binary (currently build-only).

---

## Recommended Fix Plan

### P0 — Must fix before production / public exposure

| # | Why | Files | Approach | Regression risk | Required test |
|---|---|---|---|---|---|
| P0-1 | Clients calling `/players` break | `api/rest/routes/player/get.rs`, `discord/player/state.rs` | Return `Json(Vec<Player>)`; drop `Players` wrapper | Low (only this endpoint) | `/players` returns array of N |
| P0-2 | SSRF to metadata/localhost | `engine/source/client.rs`, `media/sources/http/mod.rs` | Custom redirect policy re-validating each hop; pin resolved IP; block metadata range | Medium (some legit redirects) | redirect-to-private refused; rebind refused |
| P0-3 | Plugin/IP/pubkey state | — | (kizunalink-test comparison only) — verify `/players` fix source before deleting | — | — |

### P1 — Strongly recommended

| # | Why | Files | Approach |
|---|---|---|---|
| P1-1 | Blocking DNS stalls workers | `media/sources/http/mod.rs` | Resolve in `spawn_blocking` / pin IP |
| P1-2 | One thread-spawn failure aborts the process | `media/sources/{http,local}/mod.rs`, `youtube/reader.rs` | Replace `expect` with error to `err_tx` |
| P1-3 | Sound layers die after stop | `engine/mix/mixer.rs` | Re-enable `audio_mixer` in `add_layer` |
| P1-4 | Voice/DAVE unverified | `discord/*` | Mock-voice + two-party DAVE integration tests (§5, §10) |
| P1-5 | `/v4/info` semver/build | `stats/info.rs`, `protocol/info.rs` | Report protocol-consistent semver; add `build` |
| P1-6 | Empty `authorization` allowed | `config/mod.rs` | Reject empty token at startup |

### P2 — Improvements

- Dockerfile `HEALTHCHECK`; mount a writable `logs/` tmpfs or default logging to stdout under `read_only`.
- criterion benchmarks (mixer, decoder, opus, DAVE) + 1/10/50/100-player soak with measured RSS/CPU.
- Structured, rate-limited logging for dropped WS/event frames.
- `--locked` build already present; add `cargo audit` (deny already covers advisories).

---

## Final Verdict

- **Core playback:** **Solid.** Baseline works; edge cases handled; one robustness fix (ROBUST-001) and one mixer nit (PERF-002).
- **Lavalink compatibility:** **Good, one real defect.** `/players` shape (COMPAT-001) is the headline; info semver/build are cosmetic.
- **Discord Voice:** **Structurally correct**, live-unverified.
- **DAVE:** **Probably correct, under-tested**, lucid implementation and targeted unit tests.
- **Sources:** **Broadly implemented**, live reliability unverified; broken sources correctly gated off.
- **Security:** **Strong.** Constant-time auth, startup guard, path-traversal and SSRF guards — but the SSRF guard has a redirect/rebinding hole (SEC-001) and empty-password is unguarded.
- **Performance:** **Good design** (pooled buffers, bounded channels, zero-GC loop); no leak found; no measured benchmarks.
- **Deployment:** **Strong.** Docker/binary naming verified consistent; no `audium` leftovers; minor healthcheck/logging nits.
- **Testing:** **Good unit/protocol coverage, weak media/voice coverage.** CI is comprehensive but cannot prove production readiness alone.
- **Overall:** **7.5 / 10 — READY WITH FIXES.**

### Can `kizunalink-test` be deleted?

**YES, after porting/verifying X.**

The one item that must be confirmed before deleting is the **unique `/players` listing response-shape fix** (and its regression test) that this audit independently identified as COMPAT-001. If `kizunalink-test` returns a bare array from `GET /v4/sessions/{id}/players`, that is the change to preserve — the current KizunaLink returns the wrong wrapper. Every other improvement attributed to it (startup authorization protection, `load_search`, response-shape fixes) **already exists in KizunaLink** and need not be ported.

If, upon inspection, `kizunalink-test` contains no `/players` fix and no test the main repo lacks, then it can be deleted outright. **Do not delete it unreviewed.**

---

# Final Verification Pass

Independent second pass. Every finding below was re-derived from the **current** source tree, not trusted from the first pass. A Rust toolchain (cargo/rustc **1.99.0**, rustfmt, clippy) was installed and used. **No source code was modified** — only this Markdown file.

## Verification Environment (what was actually run)

| Command | Result |
|---|---|
| `rustup` install (stable, rustfmt+clippy) | ✅ cargo 1.99.0 / rustc 1.99.0 |
| `LIBOPUS_LIB_DIR=/tmp cargo check --workspace --all-targets` | ✅ **exit 0** (finished in 23.56s warm; no type errors) |
| `cargo fmt --all -- --check` | ❌ **exit 1 — exactly 1 file differs** (`kizunalink/kizuna-voice/media/sources/soundcloud/token.rs:27`) |
| `cargo clippy` / `cargo test` / `cargo build` | ⛔ NOT RUN — no `cmake`/`pkg-config`/`libopus` in this image; `cargo check` avoids linking, clippy/test/build do not |
| Live Discord voice / DAVE / live sources | NOT TESTED — external dependencies unavailable |

**Note on `cargo check`:** it type-checked the entire workspace *including all test targets* without a real libopus, which is valid because `cargo check` performs no link step. This proves the tree **type-checks**, not that it links/passes tests.

**FMT finding (new, CONFIRMED):** `cargo fmt --check` fails on `soundcloud/token.rs` under rustfmt 1.99 because a closure body that an older rustfmt kept multi-line is now collapsed. CI pins `dtolnay/rust-toolchain@stable` (a moving target), so CI can go red on formatting without any code change. Severity LOW (formatting only, no runtime effect).

## Verification verdict table

| ID | Finding | Verdict | Evidence (current source) | Severity | Fix Needed |
|---|---|---|---|---|---|
| COMPAT-001 | `/players` response shape | **CONFIRMED** | `player/get.rs::get_players` returns `Json(Players{players})`; `state.rs` `struct Players{players}`; docs require bare array | HIGH | YES |
| SEC-001 | HTTP SSRF redirect / DNS rebinding | **CONFIRMED** | `engine/source/client.rs::create_client` sets no redirect policy (reqwest follows ≤10); `validate_public_url` checks only the initial URL | HIGH | YES |
| SEC-002 | Empty authorization | **CONFIRMED** | `config/mod.rs::validate` rejects `"youshallnotpass"` on public bind but **not** `""` | MEDIUM | YES |
| SEC-003 | Blocking DNS | **CONFIRMED** | `http/mod.rs:155` `to_socket_addrs()` called from **sync** `can_handle` (line 190) and from `probe_metadata` (line 53) on the async runtime | MEDIUM | YES |
| ROBUST-001 | Decoder `expect()` + `panic=abort` | **PARTIAL** | Only **3** sites use `.expect()` on `thread::Builder::spawn` (`http/mod.rs:292`, `local/mod.rs:286`, `youtube/hls/mod.rs:234`); **all other sources handle spawn failure** (`tracing::error!`). Plus `routeplanner/mod.rs:47` `panic!` on bad config CIDR | MEDIUM | YES (narrowed) |
| PERF-002 | Mixer not re-enabled | **CONFIRMED** | `mixer.rs::stop_all` sets `audio_mixer.enabled=false`; `audio_mixer.add_layer` never restores it | LOW | YES |
| COMPAT-002 | `/info` semver | **CONFIRMED** | `stats/info.rs::get_info` — `semver = 1.1.0`, `major` forced ≥4 | LOW | YES |
| COMPAT-003 | Missing `version.build` | **CONFIRMED** | `protocol/info.rs::Version` has no `build` field | LOW | YES |

No finding was confirmed merely because the first report asserted it; `ROBUST-001` was **downgraded to PARTIAL** on inspection (most sources already guard the spawn).

## Confirmed Bugs

### BUG-001 — `/players` returns an object, not an array

**Status:** CONFIRMED · **Severity:** HIGH · **File:** `kizuna-server/src/api/rest/routes/player/get.rs` (`get_players`) + `kizunalink/kizuna-voice/discord/player/state.rs` (`Players`).

Traced behavior: empty session → `{"players":[]}`; one player → `{"players":[{…}]}`; N players → `{"players":[…]}` — always an object. HTTP status is `200 OK` (correct), headers correct (`Lavalink-Api-Version: 4`); only the **body shape** is wrong. Lavalink v4 and its client libraries expect a bare array.

**Minimal patch (do not apply yet):**
```rust
// get.rs — replace the final return
(StatusCode::OK, Json(players)).into_response()
// and remove `Players` from the `use` + delete/repurpose the wrapper in state.rs
```

**Regression test design (add to `kizuna-server/src/api/rest/tests.rs`):** register a session, `PATCH` to create 2 players, `GET /v4/sessions/{sid}/players`, then assert `body.is_array()`, `len == 2`, `body[0]["guildId"].is_string()`, and `body[0].get("players").is_none()` (guards against re-introducing the wrapper).

### BUG-002 — Process abort on decoder-thread spawn failure (`panic = "abort"`)

**Status:** CONFIRMED (narrowed) · **Severity:** MEDIUM · **Files:** `media/sources/http/mod.rs:292`, `media/sources/local/mod.rs:286`, `media/sources/youtube/hls/mod.rs:234`.

With `[profile.release] panic = "abort"`, a failed `std::thread::Builder::spawn` at these three sites aborts the entire process. Trigger: thread/OS-resource exhaustion under many concurrent players. Every other source already routes the error to `tracing::error!` (see the grep evidence in the table), so the fix is to make these three consistent.

**Minimal patch:** replace `.expect(…)` with `if let Err(e) = …spawn(…) { let _ = err_tx.send(format!("failed to spawn decoder thread: {e}")); }` — the `err_tx` channel already exists in each of these blocks.

**Regression test:** unit-test the error branch by asserting the function returns a decoder output whose `err_rx` yields a message when spawn is made to fail (or factor the spawn into a small helper taking a closure that can be injected).

## Confirmed Security Issues

### SEC-001 — SSRF guard is bypassable (redirects + DNS rebinding)

**Status:** CONFIRMED · **Severity:** HIGH · **Files:** `media/sources/http/mod.rs` (`validate_public_url`, `can_handle`), `engine/source/client.rs` (`create_client`), `engine/source/http/{mod,prefetcher}.rs`.

Source-level trace of the lifecycle:

```
user URL
  → reqwest::Url::parse (scheme must be http/https)
  → validate_public_url: to_socket_addrs() → reject loopback/private/link-local/multicast/unspecified (v4+v6)
  → create_client: build() with NO redirect policy  ──► reqwest follows up to 10 redirects, re-resolving DNS each hop
  → TCP connect (DNS resolved AGAIN, unvalidated)
  → final response body consumed by the prefetcher
```

Bypasses demonstrated by code inspection (no runtime exploit attempted):

| Vector | Reachable? | Why |
|---|---|---|
| `302 → http://127.0.0.1:…` | **YES** | redirects are followed without re-validation |
| DNS rebinding (public → `127.0.0.1`/`169.254.169.254`) | **YES** | validation and connect use separate lookups |
| multi-hop public→public→private | **YES** | only hop 0 is validated |
| `http://[::ffff:127.0.0.1]/` (v4-mapped v6) | **PARTIAL** | `Ipv6Addr::is_loopback()` is false for `::ffff:127.0.0.1`; it is not in the checked v6 set |
| IPv6 loopback `::1` / link-local / ULA | blocked | explicit checks exist |
| RFC1918 / `169.254/16` on the **initial** URL | blocked | explicit checks exist |
| internal DNS names resolving to private IPs | blocked **initially**, bypassable via the rows above | same root cause |
| environment proxy (`HTTP(S)_PROXY`) | **INFLUENCES** | `create_client` sets a proxy only when configured; otherwise reqwest applies default env proxies, which can resolve/route differently |

### SEC-002 — Empty `authorization` is accepted

**Status:** CONFIRMED · **Severity:** MEDIUM · **File:** `kizuna-voice/config/mod.rs::validate`.

Behavior matrix for `authorization = ""`:

| Bind address | Result |
|---|---|
| `127.0.0.1` (loopback) | allowed (only a warning path) |
| `0.0.0.0` / public IPv4 / public IPv6 | **allowed** — the default-password guard only matches `"youshallnotpass"`, so an **empty** token passes and a client sending an empty `authorization` header is authenticated as `ct_eq("","")` → true |

**Minimal patch:** in `validate()`, `if self.server.authorization.trim().is_empty() { return Err("server.authorization must not be empty".into()) }`.

**Regression test:** `validate()` returns `Err` for `authorization=""` (and for whitespace-only) regardless of bind address.

### SEC-003 — Blocking DNS on the async runtime

**Status:** CONFIRMED · **Severity:** MEDIUM. `to_socket_addrs()` (`http/mod.rs:155`) is invoked from the synchronous `SourcePlugin::can_handle` (line 190) while iterating sources inside async `load`/`resolve_track`. A slow/hostile resolver stalls a Tokio worker (a DoS lever). Fix: resolve inside `spawn_blocking`, or resolve once and pin the IP (which also closes SEC-001’s rebinding).

## Safest SSRF Fix (design)

| Option | Description | Verdict |
|---|---|---|
| **A** | Disable redirects entirely (`Policy::none()`) | Safe but breaks legitimate CDN/URL redirects many sources rely on |
| **B** | Custom `redirect::Policy` re-validating every hop (scheme + host + resolved IPs) | Necessary |
| **C** | Resolve once and **pin** the validated IP for the connection (`ClientBuilder::resolve{,_to_addrs}`), so the address validated is the address connected | Necessary |
| **D** | **B + C combined** | **Recommended** |

**Why D is safest:** it closes both root causes — *validation says public, connection goes private* is eliminated by C (single resolution, pinned), and *redirect to private* is eliminated by B (per-hop validation). Preserve TLS correctness: keep the **hostname** for SNI/cert validation and the `Host` header while pinning only the TCP address, so HTTPS certificate validation is unaffected. Handle scheme downgrades (`https→http`) explicitly, reject non-`http(s)` hops, cap hops (≤3), and re-check the pinned set (both A and AAAA). Because connection pooling could otherwise reuse a pinned socket across a later request, build a **per-request** client (or key the pool by validated host) rather than a shared long-lived client. Note the v4-mapped IPv6 gap (`::ffff:a.b.c.d`) must be unmapped and range-checked.

**Regression tests:** (1) `302 → http://127.0.0.1/` refused; (2) host resolving public-then-private refused; (3) `http://[::ffff:127.0.0.1]/` refused; (4) `https→http` redirect refused; (5) a legitimate public→public redirect still succeeds.

## Potential Risks (real paths, not confirmed exploits)

- **PR-1 — DAVE mutex contention:** `send_raw` locks the shared `DaveHandler` (`Arc<tokio::Mutex>`) for **every** 20 ms frame while control messages also lock it for MLS crypto. Correctness is fine (single mutex serializes access), but MLS work on the audio path can add jitter. No data race found.
- **PR-2 — Player write lock held across `connect_voice().await`** (`player/update.rs::handle_voice`): a concurrent `GET /players` read blocks during gateway spawn. Latency coupling only.
- **PR-3 — `stop_signal` Arc replaced per track:** `start_playback` allocates a fresh `stop_signal`; safe today because the previous monitor task is aborted first, but fragile to future reordering.
- **PR-4 — Config-time `panic!` on bad CIDR** (`routeplanner/mod.rs:47`): an invalid `route_planner.cidrs` entry aborts startup (with `panic=abort`). Admin-controlled input only.
- **PR-5 — Env-proxy influence** on the HTTP source (see SEC-001) when no proxy is configured.

## No Issue Found (inspected and sound)

- **RTP / timing:** `RTP_VERSION_BYTE=0x80`, `RTP_OPUS_PAYLOAD_TYPE=0x78` (120), `RTP_TIMESTAMP_STEP=960`, `FRAME_DURATION_MS=20`, `PCM_FRAME_SAMPLES=960`, `TARGET_SAMPLE_RATE=OPUS_SAMPLE_RATE=48_000` with a compile-time assertion. 48 kHz stereo / 20 ms / 960 samples is **exactly correct**; `RtpState::next` wraps seq/ts/nonce cleanly and is persisted across resume (`PersistentSessionState`), so reconnect does not reset the sequence.
- **`engine/frame.rs` panics** are inside `#[cfg(test)]` — not runtime-reachable.
- **Regex `.expect("valid regex")`** are compile-time literals — not user-triggerable.
- **RTP crypto mode negotiation:** only `aead_aes256_gcm_rtpsize` / `aead_xchacha20_poly1305_rtpsize`, and never a mode the server did not offer — matches Discord’s current requirement.
- **DAVE control flow:** epochs (1 vs >1), transitions (prepare/execute), buffering before external sender, readiness only on execute, reset on invalid commit/welcome, membership add/remove — all consistent; single-mutex access avoids data races. **No confirmed bug**; classified **UNDER-TESTED (live)**.
- **Session/player lifecycle:** `get_or_create_player` is atomic per key; `handle_voice_update` re-checks after `connect_voice`; `clear_player_state` uses allocation identity (`is_same`) to avoid clobbering a replacement — the classic stale-decoder races were not found.
- **Auth:** REST + WS both use `subtle::ConstantTimeEq` with Lavalink-correct 401/403.
- **Path traversal (local):** canonicalize + `starts_with(root)`, and the source refuses to register without `media_dir`.
- **Resource bounds:** body 4 MiB, WS 1 MiB, identifier 16 KiB, decodetracks 256/2 MiB, paused queue count+byte capped, rate limiter + WS message throttle.
- **Graceful shutdown:** drains sessions and aborts gateway/track tasks.
- **Build health:** `cargo check --workspace --all-targets` passes.

## Coverage Gaps (tests needed; not known bugs)

| Pri | Gap |
|---|---|
| P0 | `/players` array shape (BUG-001) |
| P0 | SSRF redirect / rebinding / v4-mapped (SEC-001) |
| P0 | empty-authorization rejection (SEC-002) |
| P1 | decoder-spawn failure path (BUG-002) |
| P1 | real playback from a local fixture (decode→mix→opus, non-silent, correct position/endTime) |
| P1 | DAVE two-party MLS key exchange + epoch-transition fixtures |
| P1 | player-state matrix (pause/seek/stop/destroy/reconnect/voice-update-while-playing) |
| P2 | source failure mid-stream → `TrackException`/`TrackStuck`/queue-advance |
| P2 | long-run soak with measured RSS and `frameStats.deficit` |

## Final Fix Roadmap

### P0 — Fix Immediately

**P0-1 · `/players` shape** — *Issue:* COMPAT-001/BUG-001. *File:* `kizuna-server/src/api/rest/routes/player/get.rs`. *Function:* `get_players`. *Root cause:* serializes a wrapper struct instead of a `Vec<Player>`. *Minimal safe fix:* return `Json(Vec<Player>)`; drop `Players`. *Side effects:* none for other endpoints; any internal caller of `Players` must be updated. *Regression test:* array shape + length + no `players` key.

**P0-2 · SSRF redirect/DNS** — *Issue:* SEC-001. *Files:* `engine/source/client.rs`, `media/sources/http/mod.rs`. *Function:* `create_client`, `validate_public_url`. *Root cause:* no per-hop validation; validation and connect resolve separately. *Minimal safe fix:* custom redirect policy (B) + pinned resolution (C), per-request client, unmap v4-mapped v6. *Side effects:* legitimate redirects must pass validation; SNI/Host must be preserved. *Regression test:* the 5 SSRF cases above + a legitimate redirect.

**P0-3 · Empty authorization** — *Issue:* SEC-002. *File:* `kizuna-voice/config/mod.rs`. *Function:* `validate`. *Root cause:* only the literal default is rejected. *Minimal safe fix:* reject empty/whitespace `authorization`. *Side effects:* none (empty was never a valid secret). *Regression test:* `validate()` errors on empty/whitespace.

**P0-4 · Process abort on spawn failure** — *Issue:* BUG-002. *Files:* `http/mod.rs`, `local/mod.rs`, `youtube/hls/mod.rs`. *Root cause:* `.expect()` on thread spawn + `panic=abort`. *Minimal safe fix:* send to `err_tx` instead of panicking. *Side effects:* a failed track yields `loadFailed` instead of killing the node (desired). *Regression test:* spawn-failure branch emits an error and does not panic.

### P1 — Fix Before Production

**P1-1 · Blocking DNS** — `media/sources/http/mod.rs`, `can_handle`/`probe_metadata`; resolve via `spawn_blocking` or pin. *Test:* slow resolver must not stall other requests.
**P1-2 · `/info` semver + `build`** — `stats/info.rs` (`get_info`), `protocol/info.rs` (`Version`); emit protocol-consistent semver and optional `build`. *Test:* `major >= 4` and semver shape.
**P1-3 · Mixer re-enable** — `engine/mix/mixer.rs` (`AudioMixer::add_layer`): set `enabled = true`. *Test:* layer added after `stop_all` is audible.
**P1-4 · Config `panic!`** — `lavalink/routeplanner/mod.rs:47`: return an error instead of `panic!`. *Test:* invalid CIDR fails startup gracefully.
**P1-5 · DAVE/voice/playback integration tests** (coverage gaps P1 above).

### P2 — Improve Later

**P2-1 · Formatting drift** — reformat `soundcloud/token.rs` (or pin the rustfmt version in CI) so `cargo fmt --check` is stable.
**P2-2 · Benchmarks** — criterion for mixer/decoder/opus/DAVE + 1/10/50/100-player soak.
**P2-3 · Dockerfile `HEALTHCHECK`** + writable `logs/` tmpfs under `read_only`.
**P2-4 · DAVE-on-audio-path contention** — evaluate a dedicated MLS/ratchet worker if jitter appears.

## `kizunalink-test` — final decision

**SAFE TO DELETE AFTER PORTING X — or SAFE TO DELETE if `X` is absent.** The repository itself is **not present in this workspace**, so it cannot be diffed here; the decision is conditional on one inspection:

- **Port if present:** a bare-array `get_players` implementation and/or an `/players` regression test (the only thing not already in KizunaLink). Exact target if porting: `kizuna-server/src/api/rest/routes/player/get.rs` + `api/rest/tests.rs`.
- **Already present in KizunaLink (do NOT port):** startup authorization guard (`config/mod.rs::validate`), `load_search` + `/v4/loadsearch`, error/empty/exception response shapes, session/player endpoint behavior.

If `kizunalink-test` contains nothing beyond the above (i.e. no `/players` fix and no missing test), it may be deleted outright.

## Bottom line (answers to the seven questions)

1. **Definitely broken:** `GET /v4/sessions/{id}/players` returns an object instead of an array (BUG-001). Nothing else is a proven functional break.
2. **Definitely insecure:** the HTTP-source SSRF guard is bypassable via redirects/DNS rebinding (SEC-001); empty `authorization` is accepted (SEC-002).
3. **Only untested:** live sources, Discord voice, DAVE key exchange, long-run playback/RTP stability, and the integration/regression tests listed as coverage gaps — **no evidence of a bug**.
4. **Fix first:** P0-1 … P0-4.
5. **How:** each P0/P1 block above gives file + function + root cause + minimal safe fix.
6. **Proof:** each block names its regression test; all are additive, none weaken existing assertions.
7. **Delete `kizunalink-test`?** Yes — after confirming/porting the `/players` array fix, or immediately if it has none.

---

## 2026-10-09 — Milestone A verification addendum (original audit retained above)

**Scope and evidence boundary.** This checkout started from `main` at
`28c0282499908a33b935a88d708a9e7a6996195b` on
`arena/9058f455-kizunalink`, clean working tree. `git fetch origin` reported
`main` still at that SHA. The specifically named uploaded “KizunaLink v2
Relevant Issues and Architecture-Gap Report.md” was not found in the workspace;
the user's 23-item list and this repository's historical audit were the available
baselines. Open PRs at start: #1 (audit) and #3 (build/SSRF repair, mergeable).
The baseline CI run 37748708409 for 28c0282 failed Formatting, Check, Clippy,
Tests, and Linux/macOS/Windows Build; Cargo Deny passed. PR #3 run 37826163602
was green **on its own SHA**, not on main. Local baseline attempts for `cargo
fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`
and `cargo build --release --workspace` each exited **127**, with
`/bin/bash: cargo: command not found`. `sudo apt-get update` could not reach
Debian's host under the sandbox network restrictions; no local Rust toolchain
was installed. These are NOT successful local runs.

The known compilation-blocker repair was cherry-picked from the already-green
open PR #3 (`f2fcf53`, now `6335046` on this branch); it restores the missing
HLS constructor brace, fixes the reqwest redirect policy API and other build
errors while retaining SSRF checks. This overlaps PR #3 and must be reconciled
before merging either PR. The subsequent media change is `66f52f7`; `985107d` adds this matrix and repairs a test signature; workflow auto-format/fix commits `3afe861` and `bebd53e` correct formatting and mechanical test lints. Neither
commit is a claim that audio/DAVE is production-ready.

### Findings tracking (source review, not live-service claims)

`Pending` below means the new tests had not yet produced a result when this
addendum was drafted. For unimplemented findings, “not run” refers to the
specified regression, not to pre-existing unrelated tests. Paths are relative
to `kizunalink/kizuna-voice/` unless prefixed `server/` (meaning `kizuna-server/src/`).

| ID | Reported defect; current file/function | Disposition and reproduction evidence | Planned change / regression | Actual regression result |
|---|---|---|---|---|
| A01 | Range validation: `media/sources/youtube/hls/fetcher.rs::fetch_segment_into`, `engine/source/{segmented,http}` | **Partially confirmed**: old 200 guard and size cap existed; 206 Content-Range/start/length were unchecked, segmented probe trusted unvalidated total. | Added strict shared validator, capped full-body HLS fallback only with declared complete length, exact body checks; scripted 206/200/malformed/truncated/oversize/403/probe tests. | 7 scripted tests passed in CI run 37875356459 (985107d); local Cargo unavailable. |
| A02 | HLS failure lost: `media/sources/youtube/hls/mod.rs::{prefetch_loop,read}` | **Partially confirmed**: prefetcher already stored error/woke reader and stopped on error; `read` consumed the error with `.take()`, subsequent read could report EOS; seek after fatal could hang. | Keep error sticky, reject seek after terminal error, bounded retry only for 429/500/502/503/504 and timeout/connect; segment 2 HTTP 500 test asserts no segment 3. | Scripted regression passed in CI 37875356459; latest HEAD still needs CI. |
| A03 | Playlist recursion: `media/sources/youtube/hls/resolver.rs::resolve_playlist_inner` | **Partially confirmed**: raw URL cycle set AND depth 8 already existed; no canonical URL or typed cycle/depth error; invalid URL not checked before fetch. | Canonicalize URL (including fragment removal), typed errors and self/A-B-A/depth/invalid/nested tests. | Scripted regression passed in CI 37875356459; latest HEAD still needs CI. |
| A04 | Stale voice readiness: `server/api/ws/opcodes.rs::handle_voice_update` and `discord/gateway/session/handler.rs::start_voice` | **Partially confirmed**: null channel disconnect already handled, but replacement aborts old task without clearing readiness first or a generation guard; old speak task can write shared flag. | Milestone C generation and bounded join; race regression. | Not run. |
| A05 | DAVE setup/readiness: `discord/crypto/dave.rs::encrypt_opus`, `discord/gateway/session/handler.rs::on_session_description` | **Confirmed**: if protocol version >0 and session not ready, `encrypt_opus` returns input unchanged; setup failure resets and start_voice still runs, then readiness set true. Transport encryption remains separate, but this is not DAVE encryption. | Milestone C fail-closed gated send and negotiated readiness; test no media before DAVE ready/failed setup. | Not run; real Discord UNTESTED. |
| A06 | Worker ownership: `engine/source/{http,segmented}.rs`, `discord/player/context.rs::stop_track` | **Partially confirmed**: stop/abort paths exist but HTTP worker handle discarded, segmented tasks spawned without owned handles and stop aborts without awaiting. | Milestone B cancellation + bounded joins, blocked-response stress and worker-count test. | Not run. |
| A07 | Untyped provider fallback: `media/sources/manager/mod.rs::{load,resolve_track}` | **Partially confirmed**: `resolve_track` returns `Result<_, String>` and load takes first matching provider; external overlapping-source fallback behavior not reproduced. | Milestone D typed failure categories, explicit fallback matrix; scripted providers. | Not run. |
| A08 | Opus reset/frame format: `engine/codec/opus_decoder.rs::reset`, `engine/frame.rs::AudioFrame`, `engine/processor.rs::run` | **Partially confirmed**: decoder already recreates at stream rate but silently ignores recreation error; frame enum lacks format, processor changes source rate on frame without rebuilding resampler. | Milestone B deterministic format/reset tests with 8/12/16/24 kHz fixtures if obtainable. | Not run. |
| A09 | Control backpressure: `engine/playback/handle.rs::seek`, decoder command sender creation | **Partially confirmed**: seeks send without overflow contract; channel ownership/maximum must be traced further; other channels are bounded. | Milestone B bound decoder commands with seek coalescing policy; repeated-seek stress. | Not run. |
| A10 | Unknown WS commands: `server/api/ws/handler.rs` text dispatch | **Confirmed**: parse failure is only logged; no protocol error/close is returned. | Milestone D deterministic compatible handling; malformed/unknown WS tests. | Not run. |
| A11 | Direct context mutations: `server/api/ws/opcodes.rs::{handle_op,handle_play}`, REST player routes | **Confirmed design gap**: WS play/stop obtain PlayerContext write lock directly; REST routes also use context. | Evaluate incremental supervisor in milestone E after foundations; concurrent REST/WS tests. | Not run. |
| A12 | Configuration parsing: `config/mod.rs::apply_env_overrides,validate` | **Partially fixed**: cherry-picked PR #3 parses env with errors; other timing/queue/size validation requires follow-up, not assumed sound. | Milestone E adversarial config table. | PR #3 CI passed for its SHA; no current-branch config regression yet. |
| A13 | Shutdown graph: `server/main.rs`, `server/session.rs::shutdown` | **Partially confirmed**: sessions are visited/cleared and Axum shuts down, but abort handles are not awaited; cleanly-shut-down log is unconditional. | Milestone E phased drain/deadlines, SIGTERM/resumable-session test. | Not run. |
| A14 | TLS handshake: `server/tls.rs::TlsListener::accept` | **Confirmed**: `acceptor.accept(tcp).await` has no timeout; one idle client can stall accept loop. | Milestone E bounded handshake; idle-TCP test. | Not run. |
| A15 | Readiness gate: `server/health.rs::health_check` | **Confirmed**: `/health` always returns 200 when callable; no draining/readiness distinction. | Milestone F separate readiness; draining test. | Not run. |
| A16 | Event queue visibility: `server/session.rs::send_message`, `discord/gateway/session/mod.rs` | **Partially confirmed**: bounded try_send and some warnings exist; overflow outcomes not consistently observable. | Milestone F counters and overflow tests. | Not run. |
| A17 | SoundCloud terminal error: `media/sources/soundcloud/reader.rs::read,prefetch_loop` | **Partially confirmed**: error is stored and surfaced once, then consumed with `.take()`; repeated read can look like EOS. | Milestone B sticky failure + segment 2 error test. | Not run. |
| A18 | RTP underruns: `discord/gateway/session/voice.rs::speak_loop` | **Unable to verify live effect**: `MissedTickBehavior::Skip` is present, but frame-clock drift/underrun impact needs timing measurement; no live voice. | Milestone C deterministic clock/load test and authorized live check. | Not run. |
| A19 | TS fallback: `media/sources/youtube/hls/mod.rs::fetch_and_demux_into` | **Confirmed**: on empty ADTS extraction it appended raw TS bytes. | Replaced with terminal error; invalid 188-byte TS bootstrap regression. SoundCloud/Twitch need independent review. | Invalid-TS regression passed in CI 37875356459; latest HEAD still needs CI. |
| A20 | Session registry/identity: `server/app_state.rs::AppState`, `server/api/rest` | **Partially confirmed**: public session maps exist; shared-token/session-ID cross-user exploitability not established by an authorization integration test. | Milestone E explicit ownership decision + two-user REST/WS test. | Not run. |
| A21 | Lifecycle metrics: `server/monitoring`, `server/health.rs` | **Partially confirmed**: existing Prometheus/stat counters exist; no verified coverage of all requested worker/voice/retry/shutdown metrics. | Milestone F bounded-cardinality metric tests. | Not run. |
| A22 | Container policy: `Dockerfile`, `docker-compose.yml` | **Partially confirmed**: non-root/read-only/cap-drop exist; no healthcheck, PID/memory limits or explicit grace period. | Milestone F health/limits after readiness works; Docker test. | Docker unavailable locally; not run. |
| A23 | Release provenance: `.github/workflows/release.yml` | **Partially confirmed**: release builds/artifact uploads exist; no verified SBOM/provenance/promotion gate for this SHA. | Milestone F checksums/SBOM/provenance plus staging/CI gate. | Not run; no release performed. |

### Changes, validation, and continuation

Changed source files: `engine/source/{range.rs,mod.rs,http/mod.rs,segmented.rs}`,
`media/sources/youtube/hls/{fetcher.rs,mod.rs,parser.rs,resolver.rs,tests.rs}`;
baseline repair (cherry-pick) additionally changed config, source client, HTTP
provider, SoundCloud token, routeplanner, and REST info/tests. The new test
module has seven deterministic local HTTP tests (range success/failure,
playlist recursion, terminal segment error, TS demux failure, HTTP/segmented
range handling and segmented probe). They are not substitutes for live audio.

**Milestone A is not complete until the latest SHA has passing format/check/
Clippy/tests/release build and the test outcomes above are recorded.** CI run
`37875356459` at `985107d6ac8d674706fa3b365e15efa50751546d`:
Formatting PASS, Check PASS, Tests PASS, Ubuntu release Build PASS, macOS
release Build PASS, Windows release Build PASS, Cargo Deny advisories PASS,
Clippy FAIL (18 test-only lints: needless `.to_vec()` and `.err().expect()`).
These lints were corrected by `bebd53e`; its bot-triggered CI is
`action_required` and did not run any jobs. The CI test log download endpoint
was unreachable from this sandbox, so the **exact count of all tests is
unavailable** even though the Tests job passed; seven new scripted test
functions are present. PR #4 remains open. A subsequent human push is required
to rerun CI on the final code; record its results below. `git diff --check` passed before the media
commit; that does not replace rustfmt or the compiler. `cargo deny check
advisories` was unavailable locally (no cargo-deny executable). No Docker
executable, local audio run, real Discord voice, or live DAVE negotiation was
performed in this addendum. No stop/replacement stress or SIGTERM shutdown test
was performed. Production decision: **NOT READY**. Next: close remaining
Milestone A gaps (in-flight cancellation and other HLS consumers), then B
(audio reset/demux/worker ownership), C (DAVE fail-closed voice generations),
D (typed fallback/protocol), E (supervision/config/TLS/sessions/shutdown), F
(operations/release/staging). Do not merge PR #4 until overlap with #3 and
security-sensitive DAVE behavior are reviewed.

### Final CI and release evidence (fill from actual runs, not assumptions)

- Final candidate at time of this entry: `bebd53e42d0690adb431509ecb70f0c5f354bf85`; **not fully validated** (its CI run 37875733643 is `action_required` because auto-fix pushed with GitHub Actions token). This documentation-only update will trigger a fresh human-authored PR CI run.
- A latest-SHA green run and test counts are required before saying Milestone A is validated; when jobs finish, append their exact results here. Runtime checks remain UNTESTED as above.
