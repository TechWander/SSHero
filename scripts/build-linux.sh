#!/usr/bin/env bash
# Build Linux x64 installers (AppImage + deb + tar.gz) on a Linux x64 host.
# Needs: cargo, node/npm, and cross toolchains for the extra backend targets:
#   sudo apt install musl-tools gcc-aarch64-linux-gnu
# (deb packaging additionally needs the fpm helper downloaded on first run.)
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/build-common.sh"

cd "$REPO"

echo "==> [1/5] backend: linux-x64 (host) + x86_64-musl (static) + aarch64"
cargo build --release -p sshero-backend
cargo build --release -p sshero-backend --target x86_64-unknown-linux-musl
cargo build --release -p sshero-backend --target aarch64-unknown-linux-gnu

build_frontend    # [2/5]
stage_spa         # [3/5]

echo "==> [4/5] stage backend binaries"
bash apps/electron/prepack.sh linux

npmi apps/electron
echo "==> [5/5] electron-builder --linux (AppImage + deb + tar.gz)"
(cd apps/electron && npx electron-builder --linux)

list_artifacts "$REPO"/apps/electron/dist/sshero-*.AppImage \
               "$REPO"/apps/electron/dist/sshero-*.deb \
               "$REPO"/apps/electron/dist/sshero-*.tar.gz
