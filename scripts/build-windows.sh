#!/usr/bin/env bash
# Cross-build the Windows x64 installer (NSIS setup + portable exe) on a Linux host.
# Needs:
#   sudo apt install mingw-w64            # x86_64-w64-mingw32-gcc cross linker
#   sudo dpkg --add-architecture i386 && sudo apt update
#   sudo apt install wine wine32          # rcedit patches exe resources under wine
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/build-common.sh"

cd "$REPO"

command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || {
  echo "missing cross linker — run: sudo apt install mingw-w64"; exit 1
}
command -v wine >/dev/null 2>&1 || {
  echo "missing wine — run: sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install wine wine32"; exit 1
}

echo "==> [1/4] backend cross-compile: x86_64-pc-windows-gnu"
cargo build --release -p sshero-backend --target x86_64-pc-windows-gnu

build_frontend    # [2/4]
stage_spa

echo "==> [3/4] stage backend binary"
bash apps/electron/prepack.sh win

npmi apps/electron
echo "==> [4/4] electron-builder --win (NSIS + portable)"
(cd apps/electron && npx electron-builder --win)

echo
echo "Note: the installer is unsigned — SmartScreen shows a one-time warning on first run."
list_artifacts "$REPO"/apps/electron/dist/sshero-*.exe
