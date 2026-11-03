import hashlib
import io
import runpy
import tempfile
import unittest
import zipfile
from pathlib import Path

api = runpy.run_path(str(Path(__file__).with_name("check-components.py")))


class ComponentsTests(unittest.TestCase):
    def test_hash_and_size_are_both_required(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "artifact"
            path.write_bytes(b"abc")
            pin = {"size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            api["verify_archive"](path, pin)
            for bad in ({**pin, "size": 2}, {**pin, "sha256": "0" * 64}):
                with self.assertRaises(ValueError):
                    api["verify_archive"](path, bad)

    def test_archive_cannot_escape_its_destination(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = root / "bad.zip"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("../escaped", b"bad")
            with self.assertRaises(ValueError):
                api["extract_zip"](path, root / "out", 3)
            self.assertFalse((root / "escaped").exists())

    def test_unpacking_preserves_upstream_name_and_internal_directory(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = root / "good.zip"
            with zipfile.ZipFile(path, "w") as archive:
                info = zipfile.ZipInfo("yt-dlp_linux")
                info.external_attr = 0o100755 << 16
                archive.writestr(info, b"exe")
                archive.writestr("_internal/yt_dlp_ejs/yt/solver/core.min.js", b"ejs")
            api["extract_zip"](path, root / "out", 6)
            self.assertEqual((root / "out/yt-dlp_linux").read_bytes(), b"exe")
            self.assertEqual((root / "out/_internal/yt_dlp_ejs/yt/solver/core.min.js").read_bytes(), b"ejs")


if __name__ == "__main__":
    unittest.main()
