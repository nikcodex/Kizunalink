#!/usr/bin/env bash
# ==============================================================================
#  🌸 KizunaLink — 1-Command Universal Linux & Termux Installer
# ==============================================================================
set -e

REPO="nikcodex/Kizunalink"
INSTALL_DIR="${HOME}/kizunalink"

echo "🌸 Installing KizunaLink Audio Core..."
ARCH=$(uname -m)

case "${ARCH}" in
  x86_64)
    TARGET="kizunalink-linux-x86_64"
    ;;
  aarch64|arm64)
    TARGET="kizunalink-linux-arm64"
    ;;
  *)
    echo "❌ Unsupported architecture: ${ARCH}"
    exit 1
    ;;
esac

mkdir -p "${INSTALL_DIR}"
cd "${INSTALL_DIR}"

echo "📥 Downloading KizunaLink binary for ${ARCH}..."
curl -sSL "https://raw.githubusercontent.com/${REPO}/main/bin/kizunalink" -o "${INSTALL_DIR}/kizunalink" || true
chmod +x "${INSTALL_DIR}/kizunalink"

if [ ! -f "${INSTALL_DIR}/config.toml" ]; then
  cat << 'EOF' > "${INSTALL_DIR}/config.toml"
[server]
host = "0.0.0.0"
port = 2333
password = "youshallnotpass"

[sources]
jiosaavn = true
spotify = true
youtube = true
EOF
fi

echo "✅ KizunaLink installed successfully to ${INSTALL_DIR}!"
echo "🚀 To start KizunaLink, run: cd ${INSTALL_DIR} && ./kizunalink"
