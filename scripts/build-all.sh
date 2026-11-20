#!/usr/bin/env bash
# Build every installer producible on THIS host:
#   Linux host -> Linux (AppImage/deb/tar.gz) + Windows (NSIS/portable)
#   macOS host -> macOS (dmg arm64 + x64)
# macOS dmg cannot be produced from Linux (hdiutil is macOS-only).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$(uname -s)" in
  Linux)
    "$HERE/build-linux.sh"
    echo
    echo "=============================================================="
    "$HERE/build-windows.sh"
    ;;
  Darwin)
    "$HERE/build-macos.sh"
    ;;
  *)
    echo "unsupported host OS: $(uname -s) (use Linux or macOS)"; exit 2
    ;;
esac

echo
echo "All done. Installers are in apps/electron/dist/ (backend-only binaries in target/)."
