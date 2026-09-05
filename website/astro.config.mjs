// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';
import { satteri } from '@astrojs/markdown-satteri';
import { baseLinks } from './src/plugins/base-links.mjs';

/**
 * Deployment target.
 *
 * GitHub Pages (project site):  https://nikcodex.github.io/Kizuna-Docs/
 *   → DOCS_SITE=https://nikcodex.github.io  DOCS_BASE=/Kizuna-Docs
 *
 * Custom domain later (e.g. https://kizunalink.dev):
 *   → DOCS_SITE=https://kizunalink.dev      DOCS_BASE=/
 *
 * Both default to the GitHub Pages values below; override via env vars in CI.
 */
const SITE = process.env.DOCS_SITE ?? 'https://nikcodex.github.io';
const BASE = process.env.DOCS_BASE ?? '/Kizuna-Docs';

const GITHUB_REPO = 'https://github.com/nikcodex/Kizunalink';
const DOCS_REPO = 'https://github.com/nikcodex/Kizuna-Docs';

export default defineConfig({
  site: SITE,
  base: BASE,
  trailingSlash: 'ignore',
  markdown: {
    processor: satteri({ mdastPlugins: [baseLinks({ base: BASE })] }),
  },
  integrations: [
    starlight({
      title: 'KizunaLink',
      description:
        'KizunaLink (絆) — a high-performance, Lavalink v4 compatible Discord audio engine written in Rust. Zero GC pauses, ~15 MB RAM, 12 audio sources, DAVE E2EE.',
      logo: {
        src: './src/assets/logo.svg',
        alt: 'KizunaLink',
      },
      favicon: '/favicon.svg',
      social: [
        { icon: 'github', label: 'GitHub', href: GITHUB_REPO },
      ],
      editLink: {
        baseUrl: `${DOCS_REPO}/edit/main/`,
      },
      lastUpdated: true,
      customCss: ['./src/styles/custom.css'],
      components: {
        Hero: './src/components/Hero.astro',
        Footer: './src/components/Footer.astro',
        Head: './src/components/Head.astro',
      },
      head: [
        {
          tag: 'meta',
          attrs: { property: 'og:image', content: `${SITE}${BASE.replace(/\/$/, '')}/og.png` },
        },
        {
          tag: 'meta',
          attrs: { name: 'twitter:card', content: 'summary_large_image' },
        },
      ],
      expressiveCode: {
        themes: ['github-dark-default', 'github-light'],
        styleOverrides: {
          borderRadius: '0.6rem',
          codeFontSize: '0.85rem',
          frames: {
            shadowColor: 'transparent',
          },
        },
        defaultProps: {
          wrap: false,
        },
      },
      sidebar: [
        {
          label: 'Getting Started',
          items: [
            { label: 'Introduction', slug: 'getting-started' },
            { label: 'Quick Start', slug: 'getting-started/quick-start' },
            { label: 'Binary Install', slug: 'getting-started/binary' },
            { label: 'Docker', slug: 'getting-started/docker' },
            { label: 'systemd Service', slug: 'getting-started/systemd' },
            { label: 'Termux / ARM64', slug: 'getting-started/termux' },
            { label: 'Build from Source', slug: 'getting-started/build-from-source' },
            { label: 'Migrating from Lavalink', slug: 'getting-started/migrating-from-lavalink' },
          ],
        },
        {
          label: 'Configuration',
          items: [
            { label: 'Overview', slug: 'configuration' },
            { label: 'config.toml Reference', slug: 'configuration/config-toml' },
            { label: 'Environment Variables', slug: 'configuration/environment' },
            { label: 'Audio Sources', slug: 'configuration/sources' },
            { label: 'Rate Limiting', slug: 'configuration/rate-limiting' },
            { label: 'Route Planner (IP Rotation)', slug: 'configuration/route-planner' },
            { label: 'Proxy', slug: 'configuration/proxy' },
            { label: 'Security', slug: 'configuration/security' },
            { label: 'Logging', slug: 'configuration/logging' },
            { label: 'Config Generator', slug: 'configuration/generator', badge: { text: 'Tool', variant: 'tip' } },
          ],
        },
        {
          label: 'Connect a Client',
          items: [
            { label: 'How it Works', slug: 'clients' },
            { label: 'JavaScript / TypeScript', slug: 'clients/javascript' },
            { label: 'Python', slug: 'clients/python' },
            { label: 'Java / Kotlin', slug: 'clients/jvm' },
            { label: 'C# / .NET', slug: 'clients/dotnet' },
            { label: 'Raw WebSocket Walkthrough', slug: 'clients/raw' },
          ],
        },
        {
          label: 'API Reference',
          items: [
            { label: 'Overview & Authentication', slug: 'api' },
            { label: 'REST API', slug: 'api/rest' },
            { label: 'WebSocket', slug: 'api/websocket' },
            { label: 'KizunaLink Extensions', slug: 'api/extensions', badge: { text: 'Kizuna', variant: 'danger' } },
            { label: 'Filters', slug: 'api/filters' },
            { label: 'Search Prefixes & Load Types', slug: 'api/search' },
            { label: 'Track Encoding', slug: 'api/track-encoding' },
            { label: 'Errors', slug: 'api/errors' },
          ],
        },
        {
          label: 'Monitoring',
          items: [
            { label: 'Health & Stats', slug: 'monitoring' },
            { label: 'Prometheus Metrics', slug: 'monitoring/prometheus' },
            { label: 'Grafana Dashboard', slug: 'monitoring/grafana' },
          ],
        },
        {
          label: 'Internals',
          items: [
            { label: 'Architecture', slug: 'internals' },
            { label: 'kizuna-voice', slug: 'internals/kizuna-voice' },
            { label: 'DAVE End-to-End Encryption', slug: 'internals/dave' },
            { label: 'DSP Pipeline', slug: 'internals/dsp' },
            { label: 'Sessions & Players', slug: 'internals/sessions' },
            { label: 'Autoplay & Queue', slug: 'internals/queue' },
          ],
        },
        {
          label: 'Help',
          items: [
            { label: 'FAQ', slug: 'help/faq' },
            { label: 'Troubleshooting', slug: 'help/troubleshooting' },
            { label: 'Contributing', slug: 'help/contributing' },
            { label: 'Changelog', slug: 'help/changelog' },
          ],
        },
      ],
      plugins: [
        starlightLinksValidator({
          errorOnRelativeLinks: false,
          errorOnInvalidHashes: false,
        }),
      ],
    }),
  ],
});
