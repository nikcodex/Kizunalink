# Live Test Runbook (Discord)

Everything that can't be proven in a sandbox gets tested here, with a real bot.
Work through it top to bottom; check boxes as you go. Total time: ~45 min.

---

## 0. What the sandbox already proved

So you don't re-test what's green:

- ✅ 201 unit/integration tests, clippy `-D warnings`, `cargo deny` advisories
- ✅ Soak: 8 sessions × 16 players (128 players), zero session/player leaks
- ✅ Full REST surface live-tested (`/health`, `/v4/info`, `/v4/stats`,
  loadtracks, decodetrack with official 4.2.2 fixture, sessions, players)
- ✅ Full WS protocol live-tested: handshake → `ready` op, playerUpdate,
  TrackStart/TrackEnd events, seek, pause, stop, destroy, **session resumption**
- ✅ Real audio decode pipeline (HTTP source → decoder → mixer → Opus frames)
- ✅ Auth semantics 401/403, rate limiting 429 + `Retry-After`, TLS termination,
  `KIZUNA_*` env overrides, `config_server` remote config
- ✅ Live search: YouTube (21 results), JioSaavn (10), SoundCloud (10),
  Gaana (10), Audius (10, prefix is **`ausearch:`**)

---

## 1. Setup (10 min)

1. Build the release binary:
   ```bash
   cargo build --release
   ```
2. Copy config and set a strong password:
   ```bash
   cp config.example.toml config.toml
   $EDITOR config.toml    # server.authorization = "<long random string>"
   ```
3. Run it: `./target/release/kizuna-server` (keep the terminal open).
4. Create a minimal bot with [lavalink-client](https://npmjs.com/lavalink-client):
   ```js
   const { LavalinkManager } = require("lavalink-client");
   const manager = new LavalinkManager({
     nodes: [{
       host: "localhost", port: 2333,
       authorization: "<your password>", secure: false,
     }],
     client: { id: process.env.CLIENT_ID, username: "TestBot" },
   });
   ```
5. Invite the bot to a test server with a voice channel you can join.

**Pass gate:** bot connects, logs show
`WebSocket connected: session=… resumed=false`.

---

## 2. Core playback (15 min)

| # | Test | How | Pass |
|---|---|---|---|
| 2.1 | Basic play | `/play never gonna give you up` | audio audible in voice channel |
| 2.2 | Position sync | let it play 60 s; bot's "now playing" progress matches real time | no drift > 1 s/min |
| 2.3 | Pause/resume | `/pause` → `/resume` | clean continuation, no glitch |
| 2.4 | Seek | `/seek 1:30` | audio jumps correctly |
| 2.5 | Volume | `/volume 50` → `/volume 100` | no clipping/distortion |
| 2.6 | Stop | `/stop` | TrackEnd `stopped` handled by bot |
| 2.7 | Queue chain | queue 3 tracks | track 1 → `finished` → track 2 starts |
| 2.8 | Replacement | start track B while A plays | A ends `replaced`, B plays |
| 2.9 | Filters | `/bassboost` or equalizer via bot | effect audible |
| 2.10 | Long session | leave one track playing 20 min | no memory creep, no stuck frames |

---

## 3. Failure & resilience (10 min)

| # | Test | How | Pass |
|---|---|---|---|
| 3.1 | Stuck track | block network mid-play (iptables drop) | TrackStuckEvent within `stuck_threshold_ms` (10 s) |
| 3.2 | Bad track | play an HTTP URL that 404s | TrackException + `loadFailed`, node survives |
| 3.3 | Node restart | Ctrl-C the server mid-play, restart | bot resumes session (60 s window) |
| 3.4 | Kill -9 | `kill -9` the node | bot reconnects; players rebuilt |
| 3.5 | Two bots | connect a second client to the same node | independent sessions/players |

---

## 4. Multi-guild soak (5 min)

With your test bot: join voice in 3+ guilds, play different tracks in each,
let them run 10 minutes. Watch:

```bash
curl -s -H "Authorization: $PW" http://localhost:2333/v4/stats | python3 -m json.tool
watch -n5 'curl -s http://localhost:2333/health'
```

**Pass:** `players == playingPlayers == 3`, RSS stable (±10 MB), audio clean in
all three channels simultaneously.

---

## 5. Source matrix (10 min)

For each provider you plan to use, with credentials configured per
[CREDENTIALS.md](./CREDENTIALS.md):

| Source | Try | Expect |
|---|---|---|
| YouTube | `ytsearch:test` + a video URL | search returns; audio plays |
| YouTube Music | `ytmsearch:test` | search returns |
| Spotify | `spsearch:test` + playlist URL | resolves → plays |
| SoundCloud | `scsearch:test` | resolves → plays |
| JioSaavn | `jssearch:test` | resolves → plays |
| Gaana | `gnsearch:test` | resolves → plays |
| Deezer | `dzsearch:test` (ARL set) | resolves → plays |
| Apple Music | a music.apple.com URL | resolves → plays |
| Audius | `ausearch:test` | resolves → plays |
| Amazon Music | `amsearch:test` | resolves → plays |
| HTTP | direct MP3 URL | plays |
| Local | `file:///path/test.wav` | plays |
| Flowery TTS | `flowery:hello world` | plays TTS |

**Pass:** search returns results AND audio is actually audible (search-only ≠
working; resolve may still fail with expired credentials).

> **Audius note:** the correct prefix is `ausearch:` / `audsearch:`
> (`README` was fixed). `audiussearch:` intentionally matches nothing.

---

## 6. DAVE / voice encryption sanity (2 min)

Not directly observable, but verify no errors:

```bash
grep -iE "dave|mls|encrypt" <server log>
```

**Pass:** no `DAVE ... failed` / `reset` spam. First DAVE key-package exchange
happens silently on join.

---

## 7. Metrics (2 min)

```toml
[metrics.prometheus]
enabled = true
```

```bash
curl -s -H "Authorization: $PW" http://localhost:2333/metrics | head -20
```

Scrape from your Prometheus/Grafana if you have one. **Pass:** gauges update
during playback (`playing_players_total`, `cpu_lavalink_load_percentage`).

---

## 8. Before you call it production

- [ ] All 5.x source rows pass with **audio audible**
- [ ] 3.3/3.4 resilience tests pass with your real bot library
- [ ] 20-minute single-track soak clean
- [ ] Password rotated from the example value
- [ ] TLS or reverse-proxy TLS in front
- [ ] [PRODUCTION.md](./PRODUCTION.md) checklist complete

---

## Filing failures

Capture in each bug report: server log tail, the exact identifier played,
`/v4/stats` output, and the bot's WS event log. Most "source broken" issues
are expired credentials — check CREDENTIALS.md first.
