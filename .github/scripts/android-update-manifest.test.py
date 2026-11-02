import hashlib
import importlib.util
import tempfile
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("android_manifest", Path(__file__).with_name("android-update-manifest.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ManifestTest(unittest.TestCase):
    def test_manifest_matches_the_exact_apk(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "app.apk"
            apk.write_bytes(b"signed apk bytes")
            feed = module.manifest(apk, 42, "main.42-aaaaaaa", "a" * 40)
            self.assertEqual(feed["size"], apk.stat().st_size)
            self.assertEqual(feed["sha256"], hashlib.sha256(apk.read_bytes()).hexdigest())
            self.assertEqual(feed["url"], "https://github.com/Zli-UoA/zoo2/releases/download/android-main/zoo-android-main-" + "a" * 40 + ".apk")
            self.assertEqual(feed["packageName"], "works.zli.zoo")
            self.assertEqual(feed["versionCode"], 42)

    def test_rejects_invalid_commit_version_and_empty_apk(self):
        with tempfile.TemporaryDirectory() as directory:
            apk = Path(directory) / "app.apk"
            apk.write_bytes(b"apk")
            for commit in ["main", "a" * 39, "../../other"]:
                with self.assertRaises(ValueError):
                    module.manifest(apk, 42, "version", commit)
            for version in [0, -1, 2**31]:
                with self.assertRaises(ValueError):
                    module.manifest(apk, version, "version", "a" * 40)
            apk.write_bytes(b"")
            with self.assertRaises(ValueError):
                module.manifest(apk, 42, "version", "a" * 40)


if __name__ == "__main__":
    unittest.main()
