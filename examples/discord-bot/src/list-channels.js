/**
 * Prints every guild the bot is in and every voice/stage channel it can see,
 * with the members currently sitting in them.
 *
 *   npm run channels
 *
 * Use it to find the id for `DISCORD_VOICE_CHANNEL_ID` when nobody is in a
 * channel for the self-test to auto-detect.
 */
import { Client, GatewayIntentBits, ChannelType } from "discord.js";

import { assertConfig, config } from "./config.js";

const VOICE_TYPES = new Set([ChannelType.GuildVoice, ChannelType.GuildStageVoice]);

assertConfig();

const client = new Client({
  intents: [GatewayIntentBits.Guilds, GatewayIntentBits.GuildVoiceStates],
});

client.once("clientReady", async () => {
  const guilds = [...client.guilds.cache.values()];
  if (!guilds.length) {
    console.log("The bot is not in any guild. Invite it first (see README.md).");
  }

  for (const guild of guilds) {
    console.log(`\nGuild  ${guild.name}  (id ${guild.id})`);

    const channels = await guild.channels.fetch();
    const voice = [...channels.values()].filter((c) => c && VOICE_TYPES.has(c.type));

    if (!voice.length) {
      console.log("  (no voice channels visible — missing View Channel permission?)");
      continue;
    }

    for (const channel of voice) {
      const occupants = channel.members.map((m) => m.user.username);
      const who = occupants.length ? `  <- ${occupants.join(", ")}` : "";
      console.log(`  ${String(channel.id).padEnd(20)} ${channel.name}${who}`);
    }
  }

  console.log(
    "\nPut the id you want to target in DISCORD_VOICE_CHANNEL_ID, or set " +
      '"voiceChannelId" in kizuna.local.json.',
  );
  await client.destroy();
  process.exit(0);
});

client.on("error", (error) => console.error("Discord error:", error.message));

client.login(config.token).catch((error) => {
  console.error(`Login failed: ${error.message}`);
  process.exit(1);
});
