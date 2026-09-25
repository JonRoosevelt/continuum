#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p continuum-app

DEST="${HOME}/.local/bin"
mkdir -p "$DEST"
install -m 0755 target/release/continuum "$DEST/continuum"
echo "installed $DEST/continuum"

"$DEST/continuum" install-service
