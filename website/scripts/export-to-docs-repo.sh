#!/usr/bin/env bash
# Copies this self-contained `website/` folder into a checkout of the
# nikcodex/Kizuna-Docs repository (the site's real home) so it can be
# committed and pushed there. Run from anywhere:
#
#   bash website/scripts/export-to-docs-repo.sh /path/to/Kizuna-Docs
#
# If the target directory doesn't exist it is cloned first.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-../Kizuna-Docs}"
REPO="https://github.com/nikcodex/Kizuna-Docs.git"

if [ ! -d "$DEST/.git" ]; then
  echo "→ cloning $REPO into $DEST"
  git clone "$REPO" "$DEST"
fi

echo "→ syncing $SRC → $DEST"
# rsync keeps .git; excludes build output and installed deps.
if command -v rsync >/dev/null 2>&1; then
  rsync -a --delete \
    --exclude '.git' --exclude 'node_modules' --exclude 'dist' --exclude '.astro' \
    "$SRC/" "$DEST/"
else
  (cd "$SRC" && tar --exclude='./node_modules' --exclude='./dist' --exclude='./.astro' -cf - .) | (cd "$DEST" && tar -xf -)
fi

cat <<MSG

Done. Next, inside $DEST:

  npm ci && npm run build      # sanity check
  git add -A
  git commit -m "docs: import KizunaLink documentation site"
  git push origin main

Then enable Pages once: GitHub → Kizuna-Docs → Settings → Pages →
"Build and deployment" → Source: GitHub Actions. The deploy workflow in
.github/workflows/deploy.yml publishes https://nikcodex.github.io/Kizuna-Docs/
on every push to main.
MSG
