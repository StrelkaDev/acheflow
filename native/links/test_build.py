import importlib.util
import json
from pathlib import Path
import plistlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("link_build", Path(__file__).with_name("build.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class BuildTests(unittest.TestCase):
    def test_signed_metadata_has_exact_separate_hosts_paths_and_identities(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "sites"
            builder.build(output, "ABCD123456", "PREFIX1234")
            for recipient in ("client", "server"):
                host = f"{recipient}.acheflow.com"
                root = output / host
                metadata = json.loads((root / ".well-known/apple-app-site-association").read_text())
                detail = metadata["applinks"]["details"][0]
                with (output / "macos" / f"{recipient}.links.entitlements").open("rb") as file:
                    entitlement = plistlib.load(file)
                self.assertEqual(detail["appIDs"], [f"PREFIX1234.com.acheflow.{recipient}.links"])
                self.assertEqual(detail["components"][0]["/"], "/fwd/*")
                self.assertEqual(entitlement["com.apple.application-identifier"], detail["appIDs"][0])
                self.assertEqual(entitlement["com.apple.developer.associated-domains"], [f"applinks:{host}"])
                self.assertIn(f'data-recipient="{recipient}"', (root / "index.html").read_text())
            config = (output / "Caddyfile").read_text()
            self.assertNotIn("https://acheflow.com", config)
            self.assertNotIn("https://app.acheflow.com", config)
            self.assertIn("frame-ancestors 'none'", config)

    def test_unsigned_output_does_not_claim_any_apple_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "sites"
            builder.build(output)
            self.assertFalse(list(output.rglob("apple-app-site-association")))
            self.assertFalse((output / "macos").exists())
            with self.assertRaises(ValueError):
                builder.build(output)

    def test_bad_or_shared_identities_are_rejected_before_writing(self):
        for options in ({"team_id": "placeholder"}, {"app_prefix": "PREFIX1234"},
                        {"client_handler_id": "bad/id"}, {"server_handler_id": "com.acheflow.client.links"}):
            with tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / "sites"
                with self.assertRaises(ValueError):
                    builder.build(output, **options)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
