#!/usr/bin/env python3
"""Build Tauri's static updater manifest from signed release artifacts."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from urllib.parse import quote


def exactly_one(root: Path, pattern: str) -> Path:
    matches = sorted(root.rglob(pattern))
    if len(matches) != 1:
        names = ", ".join(str(path.relative_to(root)) for path in matches) or "none"
        raise ValueError(f"expected exactly one {pattern} artifact, found {len(matches)}: {names}")
    return matches[0]


def unsigned_path(signature: Path) -> Path:
    if signature.suffix != ".sig":
        raise ValueError(f"not a signature file: {signature}")
    artifact = signature.with_suffix("")
    if not artifact.is_file():
        raise ValueError(f"signed artifact is missing: {artifact}")
    return artifact


def create_manifest(root: Path, tag: str, repository: str) -> dict:
    version = tag.removeprefix("v")
    if not version or not repository or "/" not in repository:
        raise ValueError("tag and owner/repository must be non-empty")

    linux_signature = exactly_one(root, "*.AppImage.sig")
    windows_signature = exactly_one(root, "*-setup.exe.sig")
    linux_artifact = unsigned_path(linux_signature)
    windows_artifact = unsigned_path(windows_signature)
    base = f"https://github.com/{repository}/releases/download/{quote(tag, safe='')}/"

    def platform(artifact: Path, signature: Path) -> dict:
        value = signature.read_text(encoding="utf-8").strip()
        if not value:
            raise ValueError(f"empty signature: {signature}")
        # GitHub stores release assets with spaces turned into dots ("Le courriel_…" is
        # served as "Le.courriel_…"), so link to that name.
        return {"url": base + quote(artifact.name.replace(" ", ".")), "signature": value}

    return {
        "version": version,
        "notes": "See the GitHub release notes for this version.",
        "platforms": {
            "linux-x86_64": platform(linux_artifact, linux_signature),
            "windows-x86_64": platform(windows_artifact, windows_signature),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--output", type=Path, default=Path("latest.json"))
    args = parser.parse_args()
    manifest = create_manifest(args.artifacts, args.tag, args.repository)
    args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
