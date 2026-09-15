/**
 * KizunaLink test bot — discord.js v14 + lavalink-client.
 *
 * Purpose: point it at a KizunaLink (or Lavalink v4) node and verify the whole
 * voice path for real: gateway login -> join voice channel -> voice state/server
 * payloads forwarded to the node -> track resolves -> audio frames reach Discord.
 *
 * Two ways to use it:
 *   npm start          interactive slash commands
 *   npm run selftest   non-interactive verification run, exits non-zero on failure
 */

import {
  Client,
  Events,
  GatewayIntentBits,
  MessageFlags,
  REST,
  Routes,
  SlashCommandBuilder,
} from "discord.js";
import { LavalinkManager } from "lavalink-client";

import { assertConfig, config, nodeOptions } from "./config.js";
import { runSelfTest } from "./selftest.js";

const log = (...args) => console.log(`[${new Date().toISOString()}]`, ...args);

assertConfig();

// ---------------------------------------------------------------------------
// Discord client
// ---------------------------------------------------------------------------

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    // Required: without it Discord never sends the bot its own VOICE_STATE_UPDATE,
    // so the node can never talk to the voice server.
    GatewayIntentBits.GuildVoiceStates,
  ],
});

// ---------------------------------------------------------------------------
// Lavalink manager
// ---------------------------------------------------------------------------

client.lavalink = new LavalinkManager({
  nodes: [nodeOptions()],
  // Handles the op-4 voice state update and forwards voice server payloads.
  sendToShard: (guildId, payload) => client.guilds.cache.get(guildId)?.shard?.send(payload),
  client: { id: config.clientId, username: "KizunaLinkTestBot" },
  autoSkip: true,
  advancedOptions: {
    // Surfaces exactly why the library refused to forward a voice update.
    enableDebugEvents: true,
    debugOptions: { noAudio: true },
  },
  playerOptions: {
    defaultSearchPlatform: "ytsearch",
    onDisconnect: { autoReconnect: true, destroyPlayer: false },
    onEmptyQueue: { destroyAfterMs: 60_000 },
  },
});

client.on(Events.Raw, (packet) => {
  // Voice state + voice server updates must reach the node or playback stalls.
  if (packet?.t === "VOICE_STATE_UPDATE" || packet?.t === "VOICE_SERVER_UPDATE") {
    log(`gateway -> node: ${packet.t}`);
  }
  client.lavalink.sendRawData(packet);
});

// ---------------------------------------------------------------------------
// Node + player event logging (this is the evidence trail for a live test)
// ---------------------------------------------------------------------------

const nodeManager = client.lavalink.nodeManager;

nodeManager.on("connect", (node) => log(`✅ node connected: ${node.options.id ?? node.id}`));
nodeManager.on("disconnect", (node) => log(`⚠️  node disconnected: ${node.options.id ?? node.id}`));
nodeManager.on("reconnecting", (node) => log(`⚠️  node reconnecting: ${node.options.id ?? node.id}`));
nodeManager.on("error", (node, error) => log(`❌ node error:`, error?.message ?? error));

client.lavalink.on("trackStart", (player, track) =>
  log(`▶️  trackStart "${track?.info?.title}" (${track?.info?.author})`),
);
client.lavalink.on("trackEnd", (player, track, payload) =>
  log(`⏹️  trackEnd "${track?.info?.title}" reason=${payload?.reason}`),
);
client.lavalink.on("queueEnd", (player, track, payload) =>
  log(`📭 queueEnd (last="${track?.info?.title}", reason=${payload?.reason})`),
);
client.lavalink.on("trackError", (player, track, payload) =>
  log(`❌ trackError "${track?.info?.title}":`, JSON.stringify(payload?.exception ?? payload)),
);
client.lavalink.on("trackStuck", (player, track, payload) =>
  log(`❌ trackStuck "${track?.info?.title}" after ${payload?.thresholdMs}ms`),
);
client.lavalink.on("playerSocketClosed", (player, payload) =>
  log(`⚠️  playerSocketClosed guild=${player.guildId} code=${payload?.code} ${payload?.reason ?? ""}`),
);
client.lavalink.on("playerDestroy", (player) => log(`🗑️  playerDestroy guild=${player.guildId}`));
client.lavalink.on("debug", (data) => log("🐞 lavalink-client:", JSON.stringify(data)));

client.on(Events.Error, (error) => log("❌ discord client error:", error?.message ?? error));
client.on(Events.Warn, (message) => log("⚠️  discord warning:", message));
process.on("unhandledRejection", (error) => log("❌ unhandledRejection:", error));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function respond(interaction, content) {
  if (interaction.replied || interaction.deferred) return interaction.editReply(content);
  return interaction.reply({ content, flags: MessageFlags.Ephemeral });
}

/** Returns an existing player, or joins the caller's voice channel and creates one. */
async function ensurePlayer(interaction) {
  const voiceChannelId = interaction.member?.voice?.channelId;
  if (!voiceChannelId) {
    await respond(interaction, "Join a voice channel first.");
    return null;
  }

  const existing = client.lavalink.getPlayer(interaction.guildId);
  if (existing) {
    // Move with the user if they switched channels.
    if (existing.voiceChannelId !== voiceChannelId) {
      existing.voiceChannelId = voiceChannelId;
      await existing.connect();
    }
    return existing;
  }

  const player = client.lavalink.createPlayer({
    guildId: interaction.guildId,
    voiceChannelId,
    textChannelId: interaction.channelId,
    selfDeaf: true,
    volume: 100,
  });
  await player.connect();
  return player;
}

/** Splits "ytsearch:foo" into { query, source } for lavalink-client. */
function parseQuery(raw) {
  // Bare URLs are passed through untouched; forcing a search prefix would break them.
  if (/^(https?|file):\/\//i.test(raw)) return { query: raw, source: undefined };
  const match = /^([a-z]{2,12}search|sprec|flowery|ytmsearch|ytsearch):(.+)$/i.exec(raw);
  if (match) return { source: match[1].toLowerCase(), query: match[2].trim() };
  return { source: "ytsearch", query: raw };
}

const humanTime = (ms) => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = String(total % 60).padStart(2, "0");
  return `${m}:${s}`;
};

// ---------------------------------------------------------------------------
// Slash commands
// ---------------------------------------------------------------------------

const commands = [
  new SlashCommandBuilder().setName("join").setDescription("Join your voice channel"),
  new SlashCommandBuilder()
    .setName("play")
    .setDescription("Search and play a track (supports ytsearch:, scsearch:, spsearch:, ...)")
    .addStringOption((o) => o.setName("query").setDescription("Query or URL").setRequired(true)),
  new SlashCommandBuilder().setName("pause").setDescription("Pause playback"),
  new SlashCommandBuilder().setName("resume").setDescription("Resume playback"),
  new SlashCommandBuilder()
    .setName("seek")
    .setDescription("Seek to a position")
    .addIntegerOption((o) =>
      o.setName("seconds").setDescription("Seconds to seek to").setRequired(true).setMinValue(0),
    ),
  new SlashCommandBuilder()
    .setName("volume")
    .setDescription("Set the volume")
    .addIntegerOption((o) =>
      o.setName("percent").setDescription("0-200").setRequired(true).setMinValue(0).setMaxValue(200),
    ),
  new SlashCommandBuilder().setName("skip").setDescription("Skip the current track"),
  new SlashCommandBuilder().setName("stop").setDescription("Stop and clear the queue"),
  new SlashCommandBuilder().setName("nowplaying").setDescription("Show the current track"),
  new SlashCommandBuilder().setName("node").setDescription("Show node stats from the server"),
  new SlashCommandBuilder().setName("leave").setDescription("Disconnect and destroy the player"),
  new SlashCommandBuilder().setName("selftest").setDescription("Run the automated voice-path verification"),
].map((c) => c.toJSON());

async function registerCommands() {
  if (!config.guildId) {
    log("DISCORD_GUILD_ID not set — skipping slash-command registration (self-test still works).");
    return;
  }
  const rest = new REST().setToken(config.token);
  await rest.put(Routes.applicationGuildCommands(client.user.id, config.guildId), { body: commands });
  log(`registered ${commands.length} guild slash commands in ${config.guildId}`);
}

client.on(Events.InteractionCreate, async (interaction) => {
  if (!interaction.isChatInputCommand()) return;
  const player = client.lavalink.getPlayer(interaction.guildId);

  try {
    switch (interaction.commandName) {
      case "join": {
        const created = await ensurePlayer(interaction);
        if (created) await respond(interaction, `Joined <#${created.voiceChannelId}>.`);
        break;
      }

      case "play": {
        const target = await ensurePlayer(interaction);
        if (!target) break;

        const raw = interaction.options.getString("query", true);
        await interaction.deferReply();
        log(`searching: ${raw}`);
        const { query, source } = parseQuery(raw);
        const result = await target.search({ query, source }, interaction.user);

        if (!result?.tracks?.length) {
          await interaction.editReply(`No results for \`${raw}\` (loadType=${result?.loadType}).`);
          break;
        }

        for (const track of result.tracks) target.queue.add(track);
        if (!target.playing && !target.paused) await target.play();

        const summary = result.playlist
          ? `Queued playlist **${result.playlist.name}** (${result.tracks.length} tracks)`
          : `Queued **${result.tracks[0].info.title}** (${humanTime(result.tracks[0].info.duration)})`;
        await interaction.editReply(summary);
        break;
      }

      case "pause":
        if (!player) return respond(interaction, "Nothing is playing.");
        await player.pause();
        await respond(interaction, "Paused.");
        break;

      case "resume":
        if (!player) return respond(interaction, "Nothing is playing.");
        await player.resume();
        await respond(interaction, "Resumed.");
        break;

      case "seek": {
        if (!player) return respond(interaction, "Nothing is playing.");
        const seconds = interaction.options.getInteger("seconds", true);
        await player.seek(seconds * 1000);
        await respond(interaction, `Seeked to ${humanTime(seconds * 1000)}.`);
        break;
      }

      case "volume": {
        if (!player) return respond(interaction, "Nothing is playing.");
        const percent = interaction.options.getInteger("percent", true);
        await player.setVolume(percent);
        await respond(interaction, `Volume set to ${percent}%.`);
        break;
      }

      case "skip":
        if (!player) return respond(interaction, "Nothing is playing.");
        await player.skip();
        await respond(interaction, "Skipped.");
        break;

      case "stop":
        if (!player) return respond(interaction, "Nothing is playing.");
        await player.stopPlaying(true);
        await respond(interaction, "Stopped and cleared the queue.");
        break;

      case "nowplaying": {
        const current = player?.queue?.current;
        if (!current) return respond(interaction, "Nothing is playing.");
        await respond(
          interaction,
          `**${current.info.title}** — ${current.info.author}\n` +
            `${humanTime(player.position)} / ${humanTime(current.info.duration)} · ` +
            `volume ${player.volume}% · ${player.paused ? "paused" : "playing"}`,
        );
        break;
      }

      case "node": {
        const node = player?.node ?? [...client.lavalink.nodeManager.nodes.values()][0];
        if (!node) return respond(interaction, "No node connected.");
        await respond(
          interaction,
          `\`\`\`\n${JSON.stringify({ id: node.options.id, alive: node.isAlive, stats: node.stats }, null, 2)}\n\`\`\``,
        );
        break;
      }

      case "leave":
        if (!player) return respond(interaction, "Not connected.");
        await player.destroy("left via /leave");
        await respond(interaction, "Left the voice channel.");
        break;

      case "selftest": {
        await interaction.deferReply();
        const report = await runSelfTest(client, {
          guildId: interaction.guildId,
          voiceChannelId: interaction.member?.voice?.channelId ?? config.voiceChannelId,
          query: config.testQuery,
          onProgress: (line) => log(`   ${line}`),
        });
        // Discord caps message content at 2000 characters.
        await interaction.editReply(report.text.slice(0, 1990));
        break;
      }

      default:
        await respond(interaction, "Unknown command.");
    }
  } catch (error) {
    log(`❌ /${interaction.commandName} failed:`, error?.stack ?? error);
    await respond(interaction, `Failed: ${error?.message ?? error}`).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

async function waitFor(predicate, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return true;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`timed out after ${timeoutMs}ms waiting for ${label}`);
}

client.once(Events.ClientReady, async () => {
  log(`logged in as ${client.user.tag} (${client.user.id})`);
  log(`connecting to node ${config.lavalink.host}:${config.lavalink.port} (secure=${config.lavalink.secure})`);

  await client.lavalink.init({ id: client.user.id, username: client.user.username });

  try {
    await waitFor(() => client.lavalink.useable, 20_000, "a usable Lavalink node");
    log("✅ node is usable — REST + WS handshake succeeded");
  } catch (error) {
    log(`❌ ${error.message}`);
    if (config.selfTest) process.exit(1);
    return;
  }

  await registerCommands().catch((error) => log("❌ command registration failed:", error?.message));

  if (config.selfTest) {
    const report = await runSelfTest(client, {
      guildId: config.guildId,
      voiceChannelId: config.voiceChannelId,
      query: config.testQuery,
      onProgress: (line) => log(`   ${line}`),
    });
    console.log(`\n${report.text}\n`);
    if (!config.keepAlive) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      process.exit(report.passed ? 0 : 1);
    }
  }
});

client.login(config.token).catch((error) => {
  log("❌ login failed:", error?.message ?? error);
  log("   check DISCORD_TOKEN, and that both Guilds + Guild Voice States intents are enabled.");
  process.exit(1);
});
