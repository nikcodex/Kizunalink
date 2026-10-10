# Live two-party DAVE v1 E2EE E2E — captured evidence

Captured (UTC): see `captured_at_utc.txt`
Tested commit: see `tested_sha.txt`
Result: PASS — `voice.media.connected_and_advancing` and human-confirmed audible playback.

## Files

| File | Contents |
|---|---|
| `tested_sha.txt` | Exact commit under test (`2a4bb4124fff6c9d8493c5a1d4244ad10dd8060b`) |
| `captured_at_utc.txt` | UTC timestamp of capture |
| `node_dave_excerpt.sanitized.log` | Node log excerpt: UDP voice ready, DAVE setup/join/READY, `speak_loop` frame counters |
| `player_state.sanitized.json` | Live `/v4/sessions/{id}/players` response with voice token/session redacted |

## What was exercised

Real Discord gateway + a real second human member joined to the test voice
channel, so Discord formed the MLS group (a lone member cannot). The node ran
the actual Lavalink v4 WS protocol against a real bot (Yuna) in a real guild.

## Key evidence (sanitized)

- UDP voice ready: `[1485248400361259170] Ready: ssrc=3983, mode=aead_aes256_gcm_rtpsize`
- `DAVE session setup (v1)` / `DAVE setup context: protocol_version=1, mls_group_id=0`
- Human joined → `DAVE adding users: [912362112620331029]`
- `DAVE commit processed (tid 0)` → `DAVE session (v1) is READY`
- Encrypted media sustained: `speak_loop: 8500 ticks, frames_sent=8214 frames_nulled=1`
- Live state: `{"state":{"connected":true,"position":177720,"ping":14},"dave":{"protocolVersion":1,"privacyCode":"719390352705397864022856999967"}}`
- Human-confirmed audible playback in channel `1549426184264355850`.

## Sanitization

Voice `token` and `sessionId` are redacted in the player-state JSON. The guild
id, channel id, user ids and track metadata are retained (non-secret test
fixtures). No bot token is present in any file.
