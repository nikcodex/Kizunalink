/**
 * Configuration for the KizunaLink test bot.
 *
 * Resolution order — first value that is set wins:
 *   1. real environment variables
 *   2. `kizuna.local.json` next to package.json (gitignored, for local testing)
 *   3. built-in defaults
 *
 * The local file exists so you can run the bot without exporting anything.
 * It is deliberately gitignored: committing a bot token to a public repo means
 * someone else can drive your bot until Discord's secret scanner revokes it.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

/** Absolute path of the optional local override file. */
export const localConfigPath = join(here, "..", "kizuna.local.json");

function loadLocalConfig() {
  if (!existsSync(localConfigPath)) return {};
  try {
    return JSON.parse(readFileSync(localConfigPath, "utf8"));
  } catch (error) {
    console.error(`Could not parse ${localConfigPath}: ${error.message}`);
    process.exit(1);
  }
}

const local = loadLocalConfig();

/** env var -> local file key -> default */
const pick = (envName, localKey, fallback = "") => process.env[envName] ?? local[localKey] ?? fallback;

const bool = (value, fallback = false) => {
  if (value === undefined || value === "") return fallback;
  return ["1", "true", "yes", "on"].includes(String(value).toLowerCase());
};

export const config = {
  token: pick("DISCORD_TOKEN", "discordToken"),
  clientId: pick("DISCORD_CLIENT_ID", "discordClientId", "0"),

  // Optional: auto-detected from the voice channel you are sitting in.
  guildId: pick("DISCORD_GUILD_ID", "guildId"),
  voiceChannelId: pick("DISCORD_VOICE_CHANNEL_ID", "voiceChannelId"),
  textChannelId: pick("DISCORD_TEXT_CHANNEL_ID", "textChannelId"),

  lavalink: {
    host: pick("LAVALINK_HOST", "lavalinkHost", "127.0.0.1"),
    port: Number(pick("LAVALINK_PORT", "lavalinkPort", 2333)),
    password: pick("LAVALINK_PASSWORD", "lavalinkPassword"),
    secure: bool(pick("LAVALINK_SECURE", "lavalinkSecure", ""), false),
    id: pick("LAVALINK_NODE_ID", "lavalinkNodeId", "kizunalink"),
  },

  /** `ytsearch:...` by default; override to test a different source. */
  testQuery: pick("TEST_QUERY", "testQuery", "ytsearch:never gonna give you up"),

  /** Set by the `--self-test` CLI flag. */
  selfTest: process.argv.includes("--self-test"),

  /** Optional: stay connected after the self-test instead of leaving. */
  keepAlive: bool(pick("KEEP_ALIVE", "keepAlive", ""), false),
};

/** Returns the node options in the shape lavalink-client expects. */
export function nodeOptions() {
  const { host, port, password, secure, id } = config.lavalink;
  return { id, host, port, authorization: password, secure };
}

/**
 * Fail fast with an actionable message instead of letting discord.js die with
 * an opaque "Used disallowed intents" or "401 Unauthorized".
 */
export function assertConfig({ requireVoiceTarget = false } = {}) {
  const problems = [];

  if (!config.token) problems.push("DISCORD_TOKEN is not set");
  if (!config.lavalink.password) problems.push("LAVALINK_PASSWORD is not set");
  if (requireVoiceTarget) {
    if (!config.guildId) problems.push("DISCORD_GUILD_ID is not set");
    if (!config.voiceChannelId) problems.push("DISCORD_VOICE_CHANNEL_ID is not set");
  }

  if (problems.length) {
    console.error("\nMissing configuration:\n" + problems.map((p) => `  - ${p}`).join("\n"));
    console.error("\nSet them as environment variables, or put them in kizuna.local.json");
    console.error("(see examples/discord-bot/README.md).\n");
    process.exit(1);
  }
}
