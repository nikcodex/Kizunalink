# KizunaLink Discord test bot

A minimal [discord.js](https://discord.js.org) v14 + [lavalink-client](https://npmjs.com/lavalink-client)
bot whose job is to prove the part unit tests cannot: **real audio reaching a real
Discord voice channel.**

Everything else about KizunaLink is already verifiable from the REST/WS surface.
This bot closes the last gap — gateway → voice state → node → Discord voice
WebSocket → Opus frames.

---

## What it verifies

| Check | Why it matters |
|---|---|
| Guild + voice channel reachable | The bot has the right intents and permissions |
| Bot has `Connect` + `Speak` | Otherwise the voice handshake never completes |
| Node is usable | REST + WS handshake against your node |
| **Voice payloads reach the node** | `VOICE_STATE_UPDATE` + `VOICE_SERVER_UPDATE` are forwarded |
| **`playerUpdate.state.connected == true`** | The node's voice WebSocket to Discord is up |
| Search resolves a track | Source plugins work end to end |
| `trackStart` event received | The node actually began feeding the pipeline |
| Position advances | Frames are being sent, not stalled |
| Pause freezes / resume restarts | Player state round-trips through the node |
| Seek lands where asked | Position control works |
| Volume applies | Filter/volume path works |
| Stop halts playback | Player teardown works |
| No `trackStuck` / `trackError` / `playerSocketClosed` | The audio path stayed healthy |

`connected` and `ping.ws` come straight from the node's `playerUpdate` payload —
they are the closest thing to "audio is flowing" you can assert without ears.

---

## Setup

### 1. Create the Discord application

1. Open <https://discord.com/developers/applications> → **New Application**.
2. **Bot** tab → your bot already exists → **Reset Token** → copy it.
   This is `DISCORD_TOKEN`. Treat it like a password.
3. You do **not** need any privileged gateway intents. This bot only uses
   `Guilds` + `Guild Voice States`, both of which are unprivileged.
4. Copy the **Application ID** from the General Information tab (useful for
   `DISCORD_CLIENT_ID`, optional).

### 2. Invite it

**OAuth2 → URL Generator**:

- Scopes: `bot`, `applications.commands`
- Bot Permissions: `View Channels`, `Send Messages`, `Embed Links`,
  `Connect`, `Speak`

Equivalent permissions integer: `3165184`. Resulting URL:

```
https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&permissions=3165184&scope=bot+applications.commands
```

### 3. Collect the ids

Enable **Settings → Advanced → Developer Mode** in Discord, then right-click and
**Copy ID**:

- the server → `DISCORD_GUILD_ID`
- the voice channel you will sit in → `DISCORD_VOICE_CHANNEL_ID`

### 4. Configure the environment

```bash
cd examples/discord-bot
npm install
```

The npm scripts load `.env` and `.env.local` (both gitignored) automatically via
Node's `--env-file-if-exists`, so you can either export the variables or drop
them in one of those files. Real environment variables always win.

Then export (or use a process manager / secret store):

| Variable | Required | Default | Notes |
|---|---|---|---|
| `DISCORD_TOKEN` | ✅ | — | Bot token from step 1 |
| `LAVALINK_PASSWORD` | ✅ | — | Must match `server.authorization` in `config.toml` |
| `DISCORD_GUILD_ID` | ❌ | auto-detected | Server id; also used for slash-command registration |
| `DISCORD_VOICE_CHANNEL_ID` | ❌ | auto-detected | Defaults to a voice channel a human is sitting in |
| `LAVALINK_HOST` | ❌ | `127.0.0.1` | Node hostname |
| `LAVALINK_PORT` | ❌ | `2333` | Node port |
| `LAVALINK_SECURE` | ❌ | `false` | `true` when `server.tls.enabled = true` |
| `DISCORD_CLIENT_ID` | ❌ | resolved after login | Only used in the constructor |
| `TEST_QUERY` | ❌ | `ytsearch:never gonna give you up` | Change when testing a source |
| `KEEP_ALIVE` | ❌ | `false` | Keep the bot connected after `--self-test` |

> **No ids needed to start:** leave `DISCORD_GUILD_ID` / `DISCORD_VOICE_CHANNEL_ID`
> unset and the self-test finds the voice channel you are currently sitting in.
> So the minimum viable setup is a token plus `LAVALINK_PASSWORD`.

---

## Run it

```bash
# Automated verification — joins, plays, exercises the player, exits 0/1
node src/index.js --self-test

# Interactive: slash commands in Discord
node src/index.js
```

Available slash commands: `/join`, `/play`, `/pause`, `/resume`, `/seek`,
`/volume`, `/skip`, `/stop`, `/nowplaying`, `/node`, `/leave`, `/selftest`.

`/node` prints live node stats straight from your server, and `/selftest` runs
the same checks as the CLI without leaving Discord.

### Running against a local KizunaLink

```bash
# terminal 1 — the node
cd ../..
KIZUNA_AUTHORIZATION=live-test ./target/debug/kizuna-server

# terminal 2 — the bot
cd examples/discord-bot
LAVALINK_PASSWORD=live-test DISCORD_TOKEN=... \
DISCORD_GUILD_ID=... DISCORD_VOICE_CHANNEL_ID=... \
node src/index.js --self-test
```

---

## A healthy run looks like this

```
[..] logged in as TestBot#1234 (123456789012345678)
[..] connecting to node 127.0.0.1:2333 (secure=false)
[..] ✅ node connected: kizunalink
[..] ✅ node is usable — REST + WS handshake succeeded
[..] registered 12 guild slash commands in 987654321
[..] gateway -> node: VOICE_STATE_UPDATE
[..] gateway -> node: VOICE_SERVER_UPDATE
[..] ▶️  trackStart "Never Gonna Give You Up" (Rick Astley)
    PASS  node voice WebSocket to Discord established  (ping.ws=42ms)
    PASS  trackStart event received
    PASS  position advances during playback  (312ms -> 8341ms)
```

If you get that far and audio is silent in the channel, the break is in the
encoder/UDP layer, not the protocol — capture `RUST_LOG=debug` from the node.

---

## Troubleshooting

| Symptom | Cause |
|---|---|
| `Used disallowed intents` | You enabled a privileged intent in code but not in the portal (this bot needs none) |
| `401 Unauthorized` on login | Bad `DISCORD_TOKEN` |
| `ECONNREFUSED` / node never usable | Node not running, or wrong `LAVALINK_PORT` |
| `connect` then immediate `disconnect` | Wrong `LAVALINK_PASSWORD` (the node answers 403 on the WS handshake) |
| Bot joins then leaves instantly | Missing `Connect` permission on that channel |
| `playerSocketClosed` code `4xxx` | Discord rejected the voice session — usually an expired or replayed voice token |
| `trackStart` never fires | Source/credentials problem — see [`docs/CREDENTIALS.md`](../../docs/CREDENTIALS.md) |
| `trackError` with `Not found` | The provider changed something; try another source |

---

## Notes

- The bot deliberately avoids `@discordjs/voice`. With Lavalink-style nodes the
  node owns the voice connection; the bot only forwards raw gateway payloads.
  Installing `@discordjs/voice` would make discord.js compete for that session.
- `GatewayIntentBits.GuildVoiceStates` is mandatory — without it Discord never
  sends the bot its own `VOICE_STATE_UPDATE`, so the node has nothing to connect
  with.
