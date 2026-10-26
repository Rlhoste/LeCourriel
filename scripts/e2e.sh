#!/usr/bin/env bash
# End-to-end tests: the real app window, from a fresh install, against local fake
# IMAP and AI servers. See e2e/fresh-install.e2e.js.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

missing=()
command -v tauri-driver   >/dev/null || missing+=("tauri-driver   →  cargo install tauri-driver --locked")
command -v WebKitWebDriver >/dev/null || missing+=("WebKitWebDriver →  sudo apt install webkit2gtk-driver")
command -v xvfb-run       >/dev/null || missing+=("xvfb-run       →  sudo apt install xvfb")
command -v openssl        >/dev/null || missing+=("openssl        →  sudo apt install openssl")
if [ ${#missing[@]} -gt 0 ]; then
  echo "End-to-end tests need:" >&2
  printf '  %s\n' "${missing[@]}" >&2
  echo "(SKIP_E2E=1 skips them explicitly.)" >&2
  exit 1
fi

[ -d node_modules/webdriverio ] || npm install --no-audit --no-fund
(cd src-tauri && cargo build --locked)

# A virtual display, so no window pops up and it also runs on CI.
xvfb-run --auto-servernum -s "-screen 0 1400x900x24" npm run test:e2e
