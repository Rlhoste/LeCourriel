#!/bin/bash
DIR="$(dirname "$0")"
cd "$DIR"

# The Linux updater can replace a running AppImage, but not a raw Cargo binary.
APPIMAGE_DIR="src-tauri/target/release/bundle/appimage"
APPIMAGE=$(find "$APPIMAGE_DIR" -maxdepth 1 -type f -name '*.AppImage' -printf '%f\n' 2>/dev/null | sort -V | tail -n 1)

if [ -n "$APPIMAGE" ]; then
  exec "$APPIMAGE_DIR/$APPIMAGE"
fi

exec ./src-tauri/target/release/courriel
