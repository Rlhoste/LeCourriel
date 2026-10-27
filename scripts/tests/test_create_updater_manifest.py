import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from create_updater_manifest import create_manifest  # noqa: E402


class UpdaterManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.linux = self.root / "linux" / "Le courriel_1.2.3_amd64.AppImage"
        self.windows = self.root / "windows" / "Le courriel_1.2.3_x64-setup.exe"
        for artifact, signature in [(self.linux, "linux-signature"), (self.windows, "windows-signature")]:
            artifact.parent.mkdir(parents=True, exist_ok=True)
            artifact.write_bytes(b"artifact")
            Path(str(artifact) + ".sig").write_text(signature)

    def tearDown(self):
        self.temp.cleanup()

    def test_builds_both_supported_platform_entries(self):
        manifest = create_manifest(self.root, "v1.2.3", "Rlhoste/LeCourriel")
        self.assertEqual(manifest["version"], "1.2.3")
        self.assertEqual(set(manifest["platforms"]), {"linux-x86_64", "windows-x86_64"})

    def test_urls_use_github_asset_names_and_point_to_the_tag(self):
        manifest = create_manifest(self.root, "v1.2.3", "Rlhoste/LeCourriel")
        linux_url = manifest["platforms"]["linux-x86_64"]["url"]
        self.assertIn("/releases/download/v1.2.3/", linux_url)
        self.assertTrue(linux_url.endswith("Le.courriel_1.2.3_amd64.AppImage"))

    def test_signature_contents_are_embedded_verbatim_without_whitespace(self):
        Path(str(self.linux) + ".sig").write_text("  signed-value\n")
        manifest = create_manifest(self.root, "1.2.3", "Rlhoste/LeCourriel")
        self.assertEqual(manifest["platforms"]["linux-x86_64"]["signature"], "signed-value")

    def test_missing_signature_is_rejected(self):
        Path(str(self.windows) + ".sig").unlink()
        with self.assertRaisesRegex(ValueError, "expected exactly one"):
            create_manifest(self.root, "v1", "Rlhoste/LeCourriel")

    def test_missing_signed_artifact_is_rejected(self):
        self.linux.unlink()
        with self.assertRaisesRegex(ValueError, "signed artifact is missing"):
            create_manifest(self.root, "v1", "Rlhoste/LeCourriel")

    def test_empty_signature_is_rejected(self):
        Path(str(self.linux) + ".sig").write_text("\n")
        with self.assertRaisesRegex(ValueError, "empty signature"):
            create_manifest(self.root, "v1", "Rlhoste/LeCourriel")

    def test_ambiguous_artifacts_are_rejected(self):
        extra = self.root / "extra.AppImage"
        extra.write_bytes(b"other")
        Path(str(extra) + ".sig").write_text("other-signature")
        with self.assertRaisesRegex(ValueError, "found 2"):
            create_manifest(self.root, "v1", "Rlhoste/LeCourriel")

    def test_command_line_writes_valid_json(self):
        output = self.root / "latest.json"
        subprocess.run([
            sys.executable, str(ROOT / "scripts/create_updater_manifest.py"),
            "--artifacts", str(self.root), "--tag", "v1.2.3",
            "--repository", "Rlhoste/LeCourriel", "--output", str(output),
        ], check=True)
        parsed = json.loads(output.read_text())
        self.assertEqual(parsed["version"], "1.2.3")


if __name__ == "__main__":
    unittest.main()
