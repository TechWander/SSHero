#!/bin/bash
# Copy the Rust backend binaries for the requested platform into apps/electron/bin
# so electron-builder can embed them via extraResources.
# Usage: ./prepack.sh linux | win | mac | all
#
# Runtime lookup (apps/electron/main.cjs) prefers the platform-arch variant
#   bin/sshero-backend-<process.platform>-<process.arch>[.exe]
# then falls back to the generic  bin/sshero-backend[.exe].
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
R="$REPO/target"
D="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/bin"
mkdir -p "$D"

copy() { # src dst
  if [ -f "$1" ]; then cp -f "$1" "$D/$2"; echo "  packaged $2 ($(du -h "$1" | cut -f1))"; else echo "  MISSING $1"; fi
}

case "${1:-linux}" in
  linux)
    # host builds land in target/release (no triple dir) for the default target
    copy "$R/release/sshero-backend"                           "sshero-backend-linux-x64"
    copy "$R/x86_64-unknown-linux-musl/release/sshero-backend" "sshero-backend-linux-x64-musl" # static, manual use
    copy "$R/aarch64-unknown-linux-gnu/release/sshero-backend" "sshero-backend-linux-arm64"
    # generic name consumed by main.cjs at runtime
    copy "$R/release/sshero-backend"                           "sshero-backend"
    ;;
  win)
    copy "$R/x86_64-pc-windows-gnu/release/sshero-backend.exe" "sshero-backend-win32-x64.exe"
    copy "$R/x86_64-pc-windows-gnu/release/sshero-backend.exe" "sshero-backend.exe"
    ;;
  mac)
    # requires building on macOS (osxcross is not available on Linux)
    copy "$R/aarch64-apple-darwin/release/sshero-backend"      "sshero-backend-darwin-arm64"
    copy "$R/x86_64-apple-darwin/release/sshero-backend"       "sshero-backend-darwin-x64"
    # generic name defaults to the host arch build (built first by build-macos.sh)
    copy "$R/aarch64-apple-darwin/release/sshero-backend"      "sshero-backend"
    ;;
  all) "$0" linux; "$0" win; "$0" mac ;;
  *) echo "unknown platform: $1 (linux|win|mac|all)"; exit 2 ;;
esac
