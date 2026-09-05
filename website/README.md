# ⛩️ Kizuna-Docs

Documentation website for [KizunaLink](https://github.com/nikcodex/Kizunalink), the Rust-powered, Lavalink-compatible Discord audio engine.

**Live:** https://nikcodex.github.io/Kizuna-Docs/

Built with [Astro](https://astro.build) + [Starlight](https://starlight.astro.build).

## Develop

```bash
npm ci
npm run dev        # http://localhost:4321/Kizuna-Docs/
```

| Script | What it does |
|---|---|
| `npm run dev` | Dev server with hot reload (binds `0.0.0.0`) |
| `npm run build` | Production build into `dist/`; **fails on any broken internal link or anchor** |
| `npm run preview` | Serve `dist/` locally |
| `npm run og` | Regenerate `public/og.png` from `src/assets/logo.svg` |
| `npm run export -- /path/to/Kizuna-Docs` | Sync this folder into a checkout of the docs repo |

Node 20+ (CI uses 22).

## Layout

```
astro.config.mjs        site/base, sidebar, component overrides, links validator
src/content/docs/       the pages (Markdown/MDX), one folder per sidebar section
src/components/         Hero, Footer, Head, Endpoint badge, ConfigGenerator
src/plugins/base-links  prefixes root-relative links with the base path at build time
src/styles/custom.css   dark/crimson theme + doc components
public/                 favicon, og image, robots.txt, Grafana dashboard JSON
scripts/                og image generator, export helper
.github/workflows/      deploy.yml → GitHub Pages
```

## Writing pages

- Links are **root-relative without the base**: write `/api/rest/#load-tracks`, never `/Kizuna-Docs/api/rest/`. The Sätteri plugin in `src/plugins/base-links.mjs` adds the base for Markdown links and for `href`/`link` props on components.
- API examples use `<Tabs syncKey="lang">` with **curl / JavaScript / Python** tabs so the reader's choice follows them across pages.
- Mark KizunaLink-only behaviour with `<Endpoint kizuna />` or the `Kizuna` badge.
- Every claim should be checkable against the engine source. Pages mention file names where it helps.
- Run `npm run build` before pushing — the links validator catches typos in anchors.

## Configuration

`astro.config.mjs` reads two environment variables so the same source can be hosted elsewhere:

| Variable | Default | Purpose |
|---|---|---|
| `DOCS_SITE` | `https://nikcodex.github.io` | Origin used for canonical URLs, sitemap and OG image |
| `DOCS_BASE` | `/Kizuna-Docs` | Path prefix (set to `/` for a custom domain) |
| `KIZUNA_VERSION` | `4.2.1` | Version shown in the hero badge (the badge also fetches the latest GitHub Release at runtime) |

## Deploying

`.github/workflows/deploy.yml` builds on every push to `main` and publishes with `actions/deploy-pages`. One-time setup in the repository: **Settings → Pages → Build and deployment → Source: GitHub Actions** (the option only appears once the repo has content).

## License

Documentation content: MIT, same as KizunaLink.
