#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOG=${COURRIEL_BUILD_LOG:-/tmp/opencode/courriel-build.log}

mkdir -p "$(dirname "$LOG")"
cd "$ROOT/src-tauri"

# Local desktop builds do not create signed updater artifacts. Keep routine compiler output in a
# log so successful builds stay quiet, but expose all diagnostics when the build fails.
if ! cargo tauri build --bundles appimage \
  --config '{"bundle":{"createUpdaterArtifacts":false}}' >"$LOG" 2>&1; then
  cat "$LOG" >&2
  exit 1
fi
