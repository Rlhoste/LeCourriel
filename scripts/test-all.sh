#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

npm test
npm run check:js
python3 -m unittest discover -s scripts/tests -v
(cd src-tauri && cargo test --locked)
git diff --check
