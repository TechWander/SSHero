#!/usr/bin/env bash
# Shared helpers for the per-platform build scripts. Sourced, not executed.
# Overridable knobs (env):
#   NPM_REGISTRY                  default npmmirror (set to your local mirror / npmjs as needed)
#   ELECTRON_MIRROR               Electron runtime zip mirror for npm postinstall + app-builder
#   ELECTRON_BUILDER_BINARIES_MIRROR  electron-builder tool binaries mirror
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[1]}")/.." && pwd)"
NPM_REGISTRY="${NPM_REGISTRY:-https://registry.npmmirror.com}"

export ELECTRON_MIRROR="${ELECTRON_MIRROR:-https://npmmirror.com/mirrors/electron/}"
export ELECTRON_BUILDER_BINARIES_MIRROR="${ELECTRON_BUILDER_BINARIES_MIRROR:-https://npmmirror.com/mirrors/electron-builder-binaries/}"

npmi() { # dir — install only when node_modules is missing (keeps reruns fast)
  if [ ! -d "$REPO/$1/node_modules" ]; then
    echo "==> npm install in $1 (registry=$NPM_REGISTRY)"
    (cd "$REPO/$1" && npm install --registry="$NPM_REGISTRY")
  else
    echo "==> npm deps present in $1 (skip install)"
  fi
}

build_frontend() {
  npmi frontend
  echo "==> frontend build (vue-tsc + vite)"
  (cd "$REPO/frontend" && npm run build)
}

stage_spa() {
  # copy the built SPA next to the electron app so electron-builder embeds it
  # as extraResources (plain dir — the Rust backend cannot read inside the asar)
  echo "==> stage SPA -> apps/electron/frontend-dist"
  rm -rf "$REPO/apps/electron/frontend-dist"
  cp -r "$REPO/frontend/dist" "$REPO/apps/electron/frontend-dist"
}

list_artifacts() { # glob...
  echo
  echo "Artifacts:"
  ls -lh "$@" 2>/dev/null || echo "  (none matched)"
}
