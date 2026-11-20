#!/usr/bin/env bash
# Build macOS dmg images (Apple silicon + Intel). MUST be run on a Mac.
# Needs:
#   xcode-select --install        # clang for the linker
#   rustup + node/npm
# The dmg is unsigned/unnotarized: right-click the app -> Open to bypass Gatekeeper.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/build-common.sh"

cd "$REPO"

echo "==> [1/5] rustup: add both darwin targets"
rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "==> [2/5] backend: darwin-arm64 + darwin-x64"
cargo build --release -p sshero-backend --target aarch64-apple-darwin
cargo build --release -p sshero-backend --target x86_64-apple-darwin

build_frontend    # [3/5]
stage_spa

echo "==> [4/5] stage backend binaries"
bash apps/electron/prepack.sh mac

npmi apps/electron
echo "==> [5/5] electron-builder --mac (arm64 + x64 dmg)"
(cd apps/electron && npx electron-builder --mac --arm64 && npx electron-builder --mac --x64)

echo
echo "Note: unsigned dmg — on first launch right-click the app and choose Open."
list_artifacts "$REPO"/apps/electron/dist/sshero-*.dmg
