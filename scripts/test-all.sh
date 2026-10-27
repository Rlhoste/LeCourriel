#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

npm test
npm run check:js
python3 -m unittest discover -s scripts/tests -v
(cd src-tauri && cargo test --locked)
# The real app window, from a fresh install (scripts/e2e.sh). SKIP_E2E=1 skips it.
if [ "${SKIP_E2E:-}" = "1" ]; then
  echo "End-to-end tests skipped (SKIP_E2E=1)."
else
  scripts/e2e.sh
fi
git diff --check
