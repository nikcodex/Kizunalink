/**
 * Automated voice-path verification.
 *
 * The sandbox can prove the REST/WS protocol surface, but only a real Discord
 * voice connection can prove the audio path. This runs that path and reports
 * exactly which link is broken if something fails.
 *
 * Why the checks below actually mean something:
 *   - `player.connected` comes straight from the node's `playerUpdate.state.connected`,
 *     which KizunaLink only sets once its voice WebSocket to Discord is established.
 *   - `player.ping.ws` comes from `playerUpdate.state.ping` — a live voice heartbeat.
 *   - `trackStart` is only emitted once the node is actually feeding the pipeline.
 *   - A stuck/errored/socket-closed event means the audio path died mid-flight.
 */

import { ChannelType, PermissionsBitField } from "discord.js";

const VOICE_CHANNEL_TYPES = new Set([ChannelType.GuildVoice, ChannelType.GuildStageVoice]);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const humanTime = (ms) => {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
};

export async function runSelfTest(client, { guildId, voiceChannelId, query, onProgress = () => {} }) {
  const results = [];
  const record = (name, ok, detail = "") => {
    results.push({ name, ok: Boolean(ok), detail });
    onProgress(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`);
    return Boolean(ok);
  };

  const finish = () => {
    const passed = results.every((r) => r.ok);
    const failed = results.filter((r) => !r.ok);
    const lines = [
      `**KizunaLink self-test: ${passed ? "✅ PASSED" : "❌ FAILED"}**`,
      "",
      ...results.map((r) => `${r.ok ? "✅" : "❌"} ${r.name}${r.detail ? ` — ${r.detail}` : ""}`),
    ];
    if (!passed) {
      lines.push("", "First failure usually points at the broken link: node reachable -> voice payloads -> voice WebSocket -> audio frames.");
    }
    return { passed, results, failed, text: lines.join("\n") };
  };

  const manager = client.lavalink;
  const events = { trackStart: null, trackEnd: null, trackStuck: null, trackError: null, socketClosed: null };

  const onStart = (_p, track) => (events.trackStart = track);
  const onEnd = (_p, track, payload) => (events.trackEnd = { track, payload });
  const onStuck = (_p, track, payload) => (events.trackStuck = { track, payload });
  const onError = (_p, track, payload) => (events.trackError = { track, payload });
  const onClosed = (_p, payload) => (events.socketClosed = payload);
  manager.on("trackStart", onStart);
  manager.on("trackEnd", onEnd);
  manager.on("trackStuck", onStuck);
  manager.on("trackError", onError);
  manager.on("playerSocketClosed", onClosed);

  const cleanup = () => {
    manager.off("trackStart", onStart);
    manager.off("trackEnd", onEnd);
    manager.off("trackStuck", onStuck);
    manager.off("trackError", onError);
    manager.off("playerSocketClosed", onClosed);
  };

  let player = null;

  try {
    // ---------------------------------------------------------------- reachability
    const { guild, channel, auto, humans } = await resolveVoiceTarget(client, { guildId, voiceChannelId });
    record(
      auto ? "voice target auto-detected" : "voice target from environment",
      true,
      `"${guild.name}" / "${channel.name}"${humans?.length ? ` — waiting: ${humans.join(", ")}` : ""}`,
    );
    record("voice channel is a joinable type", VOICE_CHANNEL_TYPES.has(channel.type), `type ${channel.type}`);

    const me = guild.members.me ?? (await guild.members.fetch(client.user.id));
    const perms = channel.permissionsFor(me);
    const canConnect = perms?.has(PermissionsBitField.Flags.Connect) ?? false;
    const canSpeak = perms?.has(PermissionsBitField.Flags.Speak) ?? false;
    record("bot has Connect + Speak", canConnect && canSpeak, `connect=${canConnect} speak=${canSpeak}`);

    record("node is usable", manager.useable, `${manager.nodeManager.nodes.size} node(s) configured`);

    // ---------------------------------------------------------------- fresh player
    const stale = manager.getPlayer(guild.id);
    if (stale) {
      await stale.destroy("self-test: resetting");
      await sleep(500);
    }

    player = manager.createPlayer({ guildId: guild.id, voiceChannelId: channel.id, selfDeaf: true, volume: 100 });
    await player.connect();
    record("voice state + server payloads sent to node", initializing(player), `voiceChannelId=${player.voiceChannelId}`);

    // ---------------------------------------------------------------- resolve
    const { query: q, source } = parseQuery(query);
    const search = await player.search({ query: q, source }, "self-test");
    const track = search?.tracks?.[0];
    record(
      "search resolved a track",
      Boolean(track),
      track ? `"${track.info.title}" by ${track.info.author} (${humanTime(track.info.duration)})` : `loadType=${search?.loadType}`,
    );
    if (!track) return finish();

    // ---------------------------------------------------------------- playback
    player.queue.add(track);
    await player.play();

    const started = await waitFor(() => events.trackStart !== null, 30_000);
    record("trackStart event received", started, started ? `playing "${track.info.title}"` : "no trackStart within 30s");

    if (started) {
      // Sample the node's RAW position (`lastPosition`) — never the client's
      // `position` getter, which extrapolates as
      // `lastPosition + (Date.now() - lastPositionChange)`. That getter keeps climbing
      // even when the node produces no audio, because playerUpdate arrives every ~5s
      // regardless, so it reports a plausible-looking rate for a completely dead
      // pipeline. Only *changes* to the raw counter count, and the rate is measured
      // between the updates that actually moved it.
      const changes = [{ at: Date.now(), pos: player.lastPosition }];
      for (let i = 0; i < 120; i++) {
        await sleep(100);
        const raw = player.lastPosition;
        if (raw !== changes[changes.length - 1].pos) {
          changes.push({ at: Date.now(), pos: raw });
        }
      }

      const firstS = changes[0];
      const lastS = changes[changes.length - 1];
      const wall = lastS.at - firstS.at;
      const advanced = lastS.pos - firstS.pos;
      const ratio = wall > 0 ? advanced / wall : 0;

      record(
        "node reports the voice connection as established",
        player.connected === true,
        `connected=${player.connected} ping.ws=${player.ping?.ws}ms`,
      );
      record(
        "audio produced at real-time rate",
        changes.length > 1 && ratio >= 0.9 && ratio <= 1.15,
        changes.length <= 1
          ? `node position never advanced (stuck at ${lastS.pos}ms) — no audio is being produced`
          : `${advanced}ms of audio in ${wall}ms wall time (${Math.round(ratio * 100)}% of real time, ${changes.length} raw updates)`,
      );

      // Pause must freeze the position; resume must let it run again.
      await player.pause();
      const pausedOk = player.paused === true;
      const frozenA = player.lastPosition;
      await sleep(6000);
      const frozenB = player.lastPosition;
      record("pause freezes playback", pausedOk && frozenB === frozenA, `${frozenA}ms -> ${frozenB}ms after 6s paused`);
      await player.resume();
      record("resume restarts playback", player.paused === false);

      // Seek somewhere sensible for the track length.
      const seekTarget = Math.min(45_000, Math.floor((track.info.duration || 60_000) / 2));
      await player.seek(seekTarget);
      // Wait longer than the playerUpdate period so this reads the node's own reported
      // position instead of a stale sample or the seek's local optimistic update.
      const seekLanded = await waitFor(() => Math.abs(player.lastPosition - seekTarget) < 4000, 7000);
      record("seek applied", seekLanded, `target=${seekTarget}ms node=${player.lastPosition}ms`);

      await player.setVolume(37);
      record("volume change applied", player.volume === 37, `volume=${player.volume}`);
      await player.setVolume(100);

      await player.stopPlaying(true);
      const halted = await waitFor(() => events.trackEnd !== null || !player.playing, 8000);
      record(
        "stop halts playback",
        halted,
        events.trackEnd ? `trackEnd reason=${events.trackEnd.payload?.reason}` : `playing=${player.playing}`,
      );
    }

    record("no stuck/error/socket-closed events", !events.trackStuck && !events.trackError && !events.socketClosed,
      events.trackStuck
        ? `trackStuck after ${events.trackStuck.payload?.thresholdMs}ms`
        : events.trackError
          ? `trackError: ${JSON.stringify(events.trackError.payload?.exception ?? {})}`
          : events.socketClosed
            ? `voice socket closed: code=${events.socketClosed.code}`
            : "clean",
    );

    return finish();
  } catch (error) {
    record("self-test threw", false, error?.message ?? String(error));
    return finish();
  } finally {
    cleanup();
    if (player) {
      await player.destroy("self-test complete").catch(() => {});
    }
  }
}

/**
 * Works out where to go without demanding ids from the user.
 *
 * Prefers a voice channel that currently has a human in it, since that is almost
 * always where someone is waiting to listen. Falls back to an explicit id.
 */
export async function resolveVoiceTarget(client, { guildId, voiceChannelId } = {}) {
  const guilds = guildId
    ? [await client.guilds.fetch(guildId).catch(() => null)].filter(Boolean)
    : [...client.guilds.cache.values()];

  if (!guilds.length) {
    throw new Error(
      guildId
        ? `guild ${guildId} is not visible to the bot — is it invited?`
        : "the bot is not in any server yet — invite it (permissions=3165184) and re-run",
    );
  }

  const isVoice = (channel) => VOICE_CHANNEL_TYPES.has(channel.type);

  if (voiceChannelId) {
    for (const guild of guilds) {
      const channel =
        guild.channels.cache.get(voiceChannelId) ??
        (await guild.channels.fetch(voiceChannelId).catch(() => null));
      if (channel && isVoice(channel)) {
        return { guild, channel, auto: false, humans: humansIn(channel) };
      }
    }
    throw new Error(`voice channel ${voiceChannelId} was not found in the guild(s) the bot can see`);
  }

  for (const guild of guilds) {
    await guild.channels.fetch().catch(() => {});
    for (const channel of guild.channels.cache.values()) {
      if (!isVoice(channel)) continue;
      const humans = humansIn(channel);
      if (humans.length) return { guild, channel, auto: true, humans };
    }
  }

  throw new Error("nobody is in a voice channel — join one and re-run (or set DISCORD_VOICE_CHANNEL_ID)");
}

const humansIn = (channel) =>
  [...(channel.members?.values() ?? [])].filter((m) => !m.user.bot).map((m) => m.user.username);

function initializing(player) {
  // connect() has sent op 4; the node is now waiting on Discord's voice payloads.
  return player.voiceChannelId !== null;
}

async function waitFor(predicate, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return true;
    await sleep(200);
  }
  return false;
}

function parseQuery(raw) {
  // Bare URLs are passed through untouched; forcing a search prefix would break them.
  if (/^(https?|file):\/\//i.test(raw)) return { query: raw, source: undefined };
  const match = /^([a-z]{2,12}search|sprec|flowery):(.+)$/i.exec(raw);
  if (match) return { source: match[1].toLowerCase(), query: match[2].trim() };
  return { source: "ytsearch", query: raw };
}
