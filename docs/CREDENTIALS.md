# Music Source Credentials Guide

How to obtain every token/credential KizunaLink can use. All keys go in
`config.toml` (never commit it) or via `KIZUNA_*` env vars / Docker secrets.

> **Rule of thumb:** sources marked **"no credentials"** work out of the box.
> Everything else degrades gracefully — search/playback works until the
> provider throttles you, then you need the credential.

---

## Quick reference

| Source | Credential | Required? | Where to get it |
|---|---|---|---|
| YouTube | PO token + visitor data, or OAuth refresh token | Recommended (heavily rate-limited without) | §YouTube |
| Spotify | `clientId` + `clientSecret` | Recommended | §Spotify |
| Deezer | ARL cookie (and/or master decryption key) | Required for playback | §Deezer |
| Apple Music | Media API token (auto-fetched otherwise) | Optional | §Apple Music |
| SoundCloud | `client_id` (auto-discovered otherwise) | Optional | §SoundCloud |
| JioSaavn | Decryption `secretKey` | Optional (search works without) | §JioSaavn |
| Gaana | none | — | — |
| Tidal | HiFi API server URL(s) | Required | §Tidal |
| Qobuz | `user_token` + `app_id` + `app_secret` | Required | §Qobuz |
| Yandex Music | OAuth `access_token` | Required | §Yandex Music |
| VK Music | `user_token` + `user_cookie` | Required | §VK Music |
| Netease | none (anonymous) | — | — |
| Amazon Music | none (anonymous) | — | — |
| Pandora | `csrf_token` | Required | §Pandora |
| Anghami | none (anonymous) | — | — |
| Audiomack | none (anonymous) | — | — |
| Audius | `app_name` (just identification) | Optional | §Audius |
| Mixcloud | none | — | — |
| Twitch | `client_id` (auto-discovered) | Optional | §Twitch |
| Reddit | none (public JSON) | — | — |
| Bandcamp / Shazam | none — **disabled by default** (provider-side blocking) | — | — |
| HTTP / Local files | none | — | — |
| Flowery TTS / Google TTS | none | — | — |
| Last.fm (mirror) | `api_key` | Required if enabled | §Last.fm |
| Lyrics (LRCLIB, Genius, Musixmatch, …) | Genius optional PAT | Optional | §Lyrics providers |

---

## YouTube

YouTube is the most anti-bot-hostile source. KizunaLink supports two levers,
mirroring the official `youtube-source` plugin:

### Option A — PO token + visitor data (recommended)

A **PO token** (proof-of-origin) is a browser-generated attestation that makes
your requests look like a real web client.

1. Open [https://www.youtube.com](https://www.youtube.com) in a browser and play
   any video (this mints a visitor session).
2. Open DevTools → **Application** → **Cookies** → `https://www.youtube.com`
   → copy the value of the **`VISITOR_INFO1_LIVE`** cookie → that is your
   **visitorData**.
3. Generate a PO token with the community tool
   [youtube-po-token-generator](https://github.com/YunzheZJU/youtube-po-token-generator)
   (runs the BotGuard script in a headless browser):
   ```bash
   git clone https://github.com/YunzheZJU/youtube-po-token-generator
   cd youtube-po-token-generator && npm install && node main.js
   ```
   The last line printed is your PO token (`pot.token`).
4. Put both in `config.toml` under the cipher/pot settings KizunaLink exposes
   (`sources.youtube.pot.token` and `sources.youtube.pot.visitorData`).

Tokens expire when the visitor cookie rotates (days–weeks). Automate rotation
if you run a busy node.

### Option B — OAuth device flow (TV client)

```toml
[sources.youtube]
get_oauth_token = true
```
Then follow the device-code prompt printed at startup the first time (same
flow as the official plugin: sign in at google.com/device with the shown code).
The refresh token is persisted; the REST route
`GET /youtube/oauth/{refreshToken}` refreshes it manually if needed.

### Remote cipher

`config.example.toml` ships with `cipher.url = "https://cipher.kikkia.dev/"`
— a shared community service that deciphers YouTube signatures. You can run
your own (`lavalink-devs`-compatible `/get_sts` + `/resolve_url` endpoints) and
optionally protect it with `cipher.token`.

---

## Spotify

1. Go to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard).
2. **Create app** — any name; redirect URI can be `http://localhost` (unused).
3. Copy **Client ID** and **Client Secret**.
4. Put them in `config.toml`:
   ```toml
   [sources.spotify]
   enabled = true
   clientId = "…"
   clientSecret = "…"
   ```
Free tier allows ~180 requests/min — plenty for metadata/search resolution.

---

## Deezer

1. Log in at [deezer.com](https://www.deezer.com) in a desktop browser.
2. Open DevTools → **Application** → **Cookies** → `https://www.deezer.com`
   → copy the **`arl`** cookie value (a long hex string).
3. ```toml
   [sources.deezer]
   enabled = true
   arls = ["your-arl-cookie-value"]
   ```
ARLs expire when you log out / change password. For **FLAC-quality** streams
you additionally need the master decryption key — community-documented value;
only use if you understand the legal implications in your jurisdiction.

---

## Apple Music

KizunaLink auto-derives the media API token from the public web player
(no credential needed). To pin your own (e.g. after a rotation):

1. Open [music.apple.com](https://music.apple.com), play any track.
2. DevTools → **Network** → any request to `/v1/catalog/...` →
   **Request Headers** → `Authorization: Bearer <token>` → copy the token.
3. ```toml
   [sources.applemusic]
   media_api_token = "eyJ..."
   ```

---

## SoundCloud

Search works with the auto-discovered `client_id`. To pin one:

1. Open [soundcloud.com](https://soundcloud.com), DevTools → **Network**.
2. Filter for `api-v2.soundcloud.com` requests → copy the `client_id`
   query parameter.
3. ```toml
   [sources.soundcloud]
   client_id = "…"
   ```

---

## JioSaavn

Search/resolve work without credentials. Official API endpoints are encrypted;
to use encrypted endpoints you need the `secretKey`:

```toml
[sources.jiosaavn]
decryption = { secretKey = "38346591" }   # community-known default
```

---

## Tidal

Tidal requires a community "HiFi API" proxy server:

```toml
[sources.tidal]
enabled = true
hifi_apis = ["http://localhost:8000"]          # at least one required
hifi_qualities = ["LOSSLESS", "HIGH", "LOW"]
```
Run your own instance of a Tidal HiFi API project and point `hifi_apis` at it.

---

## Qobuz

1. Create/log into a Qobuz account (paid tiers give higher quality).
2. Get your **user token**: log in at [play.qobuz.com](https://play.qobuz.com)
   → DevTools → **Application** → **Local Storage** → `https://play.qobuz.com`
   → `qobuz-session` → `user.auth_token`.
3. `app_id` / `app_secret` are the bundle constants of the Qobuz web player:
   open play.qobuz.com → DevTools → Sources → find `bundle.js` → search for
   `app_id` — both values are embedded there.
4. ```toml
   [sources.qobuz]
   user_token = "…"
   app_id = "…"
   app_secret = "…"
   ```

---

## Yandex Music

1. Log in at [music.yandex.com](https://music.yandex.com).
2. Go to your profile → **Settings** → copy the OAuth token shown, or visit
   `https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c6cbe6ac1c`
   (the official web-client client-id) and copy `access_token` from the URL.
3. ```toml
   [sources.yandexmusic]
   access_token = "y0_..."
   ```

---

## VK Music

1. Log in at [vk.com](https://vk.com) in a browser.
2. DevTools → **Network** → find any request → copy the full **Cookie** header.
3. Obtain the access token: DevTools → **Console** → run
   `VKWebAppGetAuthToken` via the mobile web client, or capture
   `access_token` from any `api.vk.com` request in the Network tab.
4. ```toml
   [sources.vkmusic]
   user_token = "vk1.a...."
   user_cookie = "remixsid=...; ...full cookie string..."
   ```

---

## Pandora

1. Log in at [pandora.com](https://www.pandora.com).
2. DevTools → **Application** → **Cookies** → copy **`csrftoken`**.
3. ```toml
   [sources.pandora]
   csrf_token = "..."
   ```

---

## Audius

No key needed — `app_name` is just polite identification for the public API:

```toml
[sources.audius]
app_name = "YourBotName"
```

Search prefixes: **`ausearch:`** or **`audsearch:`** (note: *not* `audiussearch:`).

---

## Twitch

```toml
[sources.twitch]
client_id = "..."   # leave blank to auto-discover from twitch.tv
```
To get one: [dev.twitch.tv/console](https://dev.twitch.tv/console) → Register
Your Application → copy the Client ID.

---

## Last.fm (mirror provider)

1. [Create an API account](https://www.last.fm/api/account/create) — instant,
   no review.
2. ```toml
   [sources.lastfm]
   api_key = "..."
   ```

---

## Lyrics providers

| Provider | Credential |
|---|---|
| LRCLIB | none |
| YouTube Music | none |
| Deezer | none |
| Netease | none |
| Musixmatch | none (public endpoints) |
| Letras | none |
| Yandex | none |
| Genius | optional access token — [genius.com/api-clients](https://genius.com/api-clients) → Create App → copy **Client Access Token** |

Enable/disable each in `[lyrics]`:
```toml
[lyrics]
lrclib = true
genius = true
musixmatch = true
# genius_token = "..."   # if your build exposes it
```

---

## Proxies

Most sources accept a per-source proxy to avoid datacenter IP blocks:

```toml
[sources.youtube]
proxy = { url = "http://proxy:8080", username = "user", password = "pass" }
```

For IPv6-heavy deployments (YouTube bans), also consider the built-in
[route planner](../README.md#-protocol--reliability) with your IPv6 block.

---

## Operational notes

- **Never commit `config.toml`.** Use `KIZUNA_*` env vars or Docker secrets in
  production (`KIZUNA_AUTHORIZATION` at minimum).
- Credentials age: ARLs/cookies/tokens expire — expect to refresh them every
  few weeks for Deezer/VK/Qobuz/Spotify-adjacent providers.
- YouTube PO tokens are the most volatile; if `ytsearch:` suddenly returns
  `empty` on a working node, rotate the PO token first.
- Bandcamp and Shazam ship **disabled by default** because their providers
  block datacenter traffic — enable at your own risk.
