#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p continuum-app

DEST="${HOME}/.local/bin"
mkdir -p "$DEST"
install -m 0755 target/release/continuum "$DEST/continuum"
echo "installed $DEST/continuum"

"$DEST/continuum" install-service

if [ "$(uname)" = "Linux" ] && [ -r /etc/ufw/ufw.conf ] && grep -q 'ENABLED=yes' /etc/ufw/ufw.conf; then
  sudo ufw allow 8770/tcp || true
  sudo ufw allow 8771/tcp || true
fi
