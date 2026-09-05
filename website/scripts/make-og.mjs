import sharp from 'sharp';
import { readFileSync } from 'node:fs';
const logo = readFileSync('src/assets/logo.svg', 'utf8');
const W = 1200, H = 630;
const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#0b0b10"/>
      <stop offset="100%" stop-color="#1a0c12"/>
    </linearGradient>
    <linearGradient id="acc" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="#ff4d6d"/>
      <stop offset="100%" stop-color="#c9184a"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.18" cy="0.5" r="0.5">
      <stop offset="0%" stop-color="#ff4d6d" stop-opacity="0.22"/>
      <stop offset="100%" stop-color="#ff4d6d" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="${W}" height="${H}" fill="url(#bg)"/>
  <rect width="${W}" height="${H}" fill="url(#glow)"/>
  <rect x="0" y="${H-8}" width="${W}" height="8" fill="url(#acc)"/>
  <circle cx="250" cy="315" r="170" fill="none" stroke="#ff4d6d" stroke-opacity="0.14" stroke-width="2"/>
  <circle cx="250" cy="315" r="205" fill="none" stroke="#ff4d6d" stroke-opacity="0.07" stroke-width="2"/>
  <text x="430" y="250" font-family="Inter, Segoe UI, Helvetica, Arial, sans-serif" font-size="84" font-weight="800" fill="#ffffff" letter-spacing="-2">KizunaLink</text>
  <text x="434" y="312" font-family="Inter, Segoe UI, Helvetica, Arial, sans-serif" font-size="29" font-weight="500" fill="#ff4d6d">Lavalink-compatible Discord audio engine in Rust</text>
  <text x="434" y="376" font-family="Inter, Segoe UI, Helvetica, Arial, sans-serif" font-size="25" fill="#b8b8c4">Drop-in for every Lavalink v4 client</text>
  <text x="434" y="414" font-family="Inter, Segoe UI, Helvetica, Arial, sans-serif" font-size="25" fill="#b8b8c4">320 kbps JioSaavn · DAVE E2EE · ~30 MB RAM · no JVM</text>
  <g font-family="ui-monospace, SFMono-Regular, Menlo, monospace" font-size="24" fill="#8f8fa3">
    <text x="434" y="488">nikcodex.github.io/Kizuna-Docs</text>
  </g>
</svg>`;
const logoPng = await sharp(Buffer.from(logo)).resize(240, 240).png().toBuffer();
await sharp(Buffer.from(svg)).png()
  .composite([{ input: logoPng, left: 130, top: 195 }])
  .toFile('public/og.png');
console.log('ok');
