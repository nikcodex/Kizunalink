#!/usr/bin/env python3
"""Real-client E2E regression: pause/resume mid tape-stop ramp.

Drives a real Discord bot through discord.py + wavelink against a running
KizunaLink node, then hammers `pause(); resume()` fast enough that the resume
lands *inside* the tape stop-ramp window. That is the exact race that used to
pin `TapeEffect` at its 0.01 rate floor, so the player reported PLAYING while
the channel stayed silent forever.

The pass/fail gate is behavioural: after the burst the player must still be
unpaused, still hold a track, and its reported position must keep advancing. A
pinned tape stops advancing position and emits nothing.

This exercises the real client library and the real wire protocol; it is not a
unit test and needs live Discord credentials and a voice channel with a second
member (Discord only forms the DAVE MLS group when someone else is present, and
the node gates the RTP send loop on DAVE readiness).

Environment:
  DISCORD_TOKEN         required; bot token
  GUILD                 guild id (default 1558468405604913235)
  VOICE_CHANNEL         voice channel id (default 1558469631898034246)
  KIZUNA_HOST           node host:port (default 127.0.0.1:2333)
  KIZUNA_AUTHORIZATION  node auth (default local-dev-secret-change-me-2f9b)
  E2E_TRACK             explicit identifier, e.g. jssearch:Kesariya or
                        file:///tmp/kizuna_bot/media/tone.wav
  E2E_QUERY             search query used when E2E_TRACK is unset (default Kesariya)
  BURST                 number of pause/resume pairs (default 6)
  BURST_GAP_MS          delay between pause and resume, i.e. where in the ramp
                        the resume lands (default 120; ramp is 500 ms)
  HOLD_SECS             extra listening window after the gate (default 0)
  AUDIBLE_WAIT_SECS     wait before the burst for a second member to join and
                        DAVE to become ready (default 90)

Exit code 0 when the gate passes, 1 otherwise (2 when inconclusive: no audio
path, e.g. nobody joined so DAVE never formed).
"""
import asyncio
import json
import os
import sys
import time
import urllib.parse
import urllib.request

import discord
import wavelink

TOKEN = os.environ["DISCORD_TOKEN"]
GUILD_ID = int(os.environ.get("GUILD", "1558468405604913235"))
VOICE_CHANNEL_ID = int(os.environ.get("VOICE_CHANNEL", "1558469631898034246"))
NODE_HOST = os.environ.get("KIZUNA_HOST", "127.0.0.1:2333")
AUTH = os.environ.get("KIZUNA_AUTHORIZATION", "local-dev-secret-change-me-2f9b")
E2E_TRACK = os.environ.get("E2E_TRACK", "")
E2E_QUERY = os.environ.get("E2E_QUERY", "Kesariya")
BURST = int(os.environ.get("BURST", "6"))
BURST_GAP_MS = int(os.environ.get("BURST_GAP_MS", "120"))
HOLD_SECS = int(os.environ.get("HOLD_SECS", "0"))
AUDIBLE_WAIT_SECS = int(os.environ.get("AUDIBLE_WAIT_SECS", "90"))

RESULTS = []
EVENTS = {"start": None, "end": None}


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def record(name, verdict, detail=""):
    if isinstance(verdict, bool):
        verdict = "PASS" if verdict else "FAIL"
    RESULTS.append((name, verdict, detail))
    log(f"{verdict:<4} :: {name} :: {detail}")


def rest_get(path):
    req = urllib.request.Request(f"http://{NODE_HOST}{path}")
    req.add_header("Authorization", AUTH)
    req.add_header("User-Agent", "KizunaLink-E2E")
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read().decode())


def rest_loadtracks(session_id, identifier):
    return rest_get(f"/v4/loadtracks?identifier={urllib.parse.quote(identifier)}")


def player_json(session_id):
    """Raw player JSON; None when the player is gone (404)."""
    try:
        return rest_get(f"/v4/sessions/{session_id}/players/{GUILD_ID}")
    except Exception:
        return None


def position_of(p):
    return (p or {}).get("state", {}).get("position")


def dave_ready(p):
    return bool((p or {}).get("dave", {}).get("privacyCode"))


intents = discord.Intents.default()
intents.voice_states = True
client = discord.Client(intents=intents)


@client.event
async def on_wavelink_track_start(payload):
    EVENTS["start"] = payload.track
    log(f"TrackStartEvent: {payload.track.title!r}")


@client.event
async def on_wavelink_track_end(payload):
    EVENTS["end"] = payload.reason
    log(f"TrackEndEvent: reason={payload.reason}")


async def resolve_track(node):
    identifier = E2E_TRACK or f"jssearch:{E2E_QUERY}"
    res = rest_loadtracks(node.session_id, identifier)
    load_type = res.get("loadType")
    data = res.get("data")
    if load_type == "track" and data:
        return wavelink.Playable(data), identifier
    if load_type == "search" and data:
        return wavelink.Playable(data[0]), identifier
    raise RuntimeError(
        f"loadtracks({identifier!r}) returned loadType={load_type} "
        f"exception={res.get('exception')}"
    )


async def wait_for(cond, timeout, interval=0.5):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if cond():
            return True
        await asyncio.sleep(interval)
    return False


async def main():
    await client.start(TOKEN)


async def wait_audio_path(node):
    """Wait until the node reports a DAVE-ready player with advancing position,
    which proves the RTP send loop is actually emitting frames."""
    t0 = time.time()
    while time.time() - t0 < AUDIBLE_WAIT_SECS:
        p = player_json(node.session_id)
        if dave_ready(p) and (position_of(p) or 0) > 0:
            return True
        await asyncio.sleep(1)
    return False


async def run_suite():
    guild = client.get_guild(GUILD_ID)
    channel = guild.get_channel(VOICE_CHANNEL_ID)
    record("discord.guild+channel.resolve", channel is not None,
           f"guild={getattr(guild, 'name', None)!r} channel={getattr(channel, 'name', None)!r}")
    if channel is None:
        return 1

    node = wavelink.Node(uri=f"http://{NODE_HOST}", password=AUTH)
    await wavelink.Pool.connect(nodes=[node], client=client, cache_capacity=None)
    got_session = await wait_for(lambda: node.session_id is not None, 10, 0.2)
    record("lavalink.node.connect", bool(node.session_id) and got_session,
           f"uri={node.uri} session_id={node.session_id}")
    if not node.session_id:
        return 1

    player = await channel.connect(cls=wavelink.Player, self_deaf=False)
    record("discord.voice.connect", player.connected, f"channel={channel.name!r}")

    track, ident = await resolve_track(node)
    await player.play(track)
    record("track.play", True, f"identifier={ident} track={track.title!r}")

    if not await wait_audio_path(node):
        record("audio.path.ready", "SKIP",
               f"no DAVE-ready audio path after {AUDIBLE_WAIT_SECS}s; "
               "put a second member in the voice channel")
        await player.disconnect()
        _report()
        return 2

    # Baseline: position advances while playing.
    p1 = player_json(node.session_id)
    await asyncio.sleep(2)
    p2 = player_json(node.session_id)
    baseline = (position_of(p2) or 0) > (position_of(p1) or 0)
    record("baseline.position_advancing", "PASS" if baseline else "FAIL",
           f"{position_of(p1)} -> {position_of(p2)}")

    # The race: resume inside the stop-ramp window, repeated.
    log(f"BURST: {BURST}x pause/resume with {BURST_GAP_MS}ms gap")
    paused_ok = resumed_ok = True
    for _ in range(BURST):
        await player.pause(True)
        await asyncio.sleep(BURST_GAP_MS / 1000)
        p = player_json(node.session_id)
        paused_ok = paused_ok and bool(p and p.get("paused"))
        await player.pause(False)
        await asyncio.sleep(BURST_GAP_MS / 1000)
        p = player_json(node.session_id)
        resumed_ok = resumed_ok and bool(p and not p.get("paused"))
    record("burst.pause_resume", "PASS" if (paused_ok and resumed_ok) else "FAIL",
           f"paused_ok={paused_ok} resumed_ok={resumed_ok}")

    # Gate: still playing, still has a track, and position keeps advancing.
    await asyncio.sleep(1)
    p3 = player_json(node.session_id)
    await asyncio.sleep(3)
    p4 = player_json(node.session_id)
    still_playing = bool(p4 and not p4.get("paused") and p4.get("track"))
    advancing = (position_of(p4) or 0) > (position_of(p3) or 0)
    title = (p4 or {}).get("track", {}).get("info", {}).get("title")
    record("post_burst.still_playing", "PASS" if still_playing else "FAIL",
           f"paused={(p4 or {}).get('paused')} track={title!r}")
    record("post_burst.position_advancing", "PASS" if advancing else "FAIL",
           f"{position_of(p3)} -> {position_of(p4)}  (pinned tape = no advance)")

    if HOLD_SECS > 0:
        log(f"HOLD {HOLD_SECS}s for audible confirmation")
        await asyncio.sleep(HOLD_SECS)

    await player.disconnect()
    _report()
    return 1 if any(v == "FAIL" for _, v, _ in RESULTS) else 0


def _report():
    passed = sum(1 for _, v, _ in RESULTS if v == "PASS")
    failed = sum(1 for _, v, _ in RESULTS if v == "FAIL")
    skipped = sum(1 for _, v, _ in RESULTS if v == "SKIP")
    print("\n" + "=" * 64)
    print(f"RESULT: {passed} PASS / {failed} FAIL / {skipped} SKIP  (of {len(RESULTS)})")
    for n, v, d in RESULTS:
        print(f"  {v:<4} {n} :: {d}")
    print("=" * 64)
    with open("/tmp/kizuna_e2e_results.json", "w") as f:
        json.dump([{"name": n, "verdict": v, "detail": d} for n, v, d in RESULTS], f, indent=2)


@client.event
async def on_ready():
    log(f"READY as {client.user} (id {client.user.id})")
    if not getattr(client, "_ran", False):
        client._ran = True
        try:
            code = await run_suite()
        except Exception as e:
            record("suite.completed", "FAIL", f"unhandled: {type(e).__name__}: {e}")
            _report()
            code = 1
        await client.close()
        sys.exit(code)


if __name__ == "__main__":
    asyncio.run(main())
