"""Run with python3 -m unittest discover -s native/macos -p 'test_*.py'."""
import json
from pathlib import Path
import tempfile
import unittest

from package import copy_extensions, verify_version


class PackageTests(unittest.TestCase):
    def test_wrong_binary_version_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "server with spaces"
            binary.write_text("#!/bin/sh\nprintf 'acheflow-server 0.0.0\\n'\n")
            binary.chmod(0o755)
            with self.assertRaisesRegex(ValueError, "version"):
                verify_version(binary, "acheflow-server", "0.1.3")
            binary.write_text("#!/bin/sh\nprintf 'acheflow-server 0.1.3\\n'\n")
            verify_version(binary, "acheflow-server", "0.1.3")

    def test_only_declared_extension_artifacts_are_distributed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source" / "provider"
            source.mkdir(parents=True)
            manifest = {"id": "provider", "module": "guest.wasm", "execution_side": "client",
                        "authentication": {"browser_module": "auth.js"}}
            (source / "manifest.json").write_text(json.dumps(manifest))
            (source / "guest.wasm").write_bytes(b"\0asm\x01\0\0\0")
            (source / "auth.js").write_text("export default {};")
            (source / ".env").write_text("private test fixture")
            copy_extensions(root / "source", root / "output")
            self.assertEqual({p.name for p in (root / "output/provider").iterdir()},
                             {"manifest.json", "guest.wasm", "auth.js"})

    def test_missing_and_escaping_extension_modules_fail(self):
        for module in ["missing.wasm", "../escape.wasm"]:
            with self.subTest(module=module), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / "source/provider"
                source.mkdir(parents=True)
                (source / "manifest.json").write_text(json.dumps(
                    {"id": "provider", "module": module, "execution_side": "server"}))
                with self.assertRaises((ValueError, FileNotFoundError)):
                    copy_extensions(root / "source", root / "output")


if __name__ == "__main__":
    unittest.main()
