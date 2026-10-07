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

**NOT TESTED — build toolchain unavailable in the audit environment.**

```
$ cargo --version   → cargo: command not found
$ rustc --version   → rustc: command not found
$ cmake --version   → cmake: command not found
$ pkg-config --modversion opus → pkg-config: command not found
$ which rustup → (none)
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
