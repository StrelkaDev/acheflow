"""Run with python3 -m unittest discover -s native/macos -p 'test_*.py'."""
import json
import plistlib
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import package
from package import copy_extensions, verify_version


class PackageTests(unittest.TestCase):
    def test_app_output_path_is_not_nested_inside_another_app(self):
        self.assertEqual(package.app_path(Path('/Applications/Acheflow.app')),
                         Path('/Applications/Acheflow.app'))
        self.assertEqual(package.app_path(Path('/tmp/distribution')),
                         Path('/tmp/distribution/Acheflow.app'))

    def test_rust_tools_are_found_without_cargo_on_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            tool = home / "bin/trunk"
            tool.parent.mkdir()
            tool.write_text("#!/bin/sh\n")
            tool.chmod(0o755)
            with patch.dict("os.environ", {"CARGO_HOME": str(home), "PATH": "/usr/bin:/bin"}):
                self.assertEqual(package.rust_tool("trunk"), str(tool))
                environment = package.build_environment()
                self.assertEqual(environment["PATH"].split(":")[0], str(home / "bin"))

    def test_registration_uses_the_final_absolute_app_path(self):
        with tempfile.TemporaryDirectory(prefix="app with spaces ") as temporary:
            app = Path(temporary) / "Acheflow.app"
            with patch("package.subprocess.run") as run:
                package.register_links(app)
                command = run.call_args.args[0]
                self.assertEqual(command[-2:], ["-f", str(app.resolve())])
                self.assertEqual(Path(command[0]).name, "lsregister")
                self.assertTrue(run.call_args.kwargs["check"])

    def test_bundle_registers_links_on_its_only_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "acheflow"
            binary.write_bytes(b"isolated package fixture")
            app = root / "One executable.app"
            subprocess.run([sys.executable, str(Path(__file__).with_name("build-dev-app.py")),
                            "--output", str(app), "--binary", str(binary)],
                           check=True, capture_output=True)
            contents = app / "Contents"
            self.assertEqual({p.name for p in (contents / "MacOS").iterdir()}, {"acheflow"})
            self.assertFalse((contents / "Helpers").exists())
            info = plistlib.loads((contents / "Info.plist").read_bytes())
            self.assertEqual(info["CFBundleExecutable"], "acheflow")
            self.assertEqual(info["CFBundleURLTypes"][0]["CFBundleURLSchemes"], ["acheflow"])
            self.assertIn("NSUserActivityTypeBrowsingWeb", info["NSUserActivityTypes"])
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
