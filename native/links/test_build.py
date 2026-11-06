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
    def test_app_domain_has_isolated_callback_assets_and_does_not_claim_the_browser_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "sites"
            builder.build(output, "ABCD123456")
            root = output / "app.acheflow.com"
            self.assertTrue((root / "forward.html").is_file())
            self.assertFalse((root / "index.html").exists())
            self.assertTrue((root / "_acheflow-links/entry.mjs").is_file())
            detail = json.loads((root / ".well-known/apple-app-site-association").read_text())["applinks"]["details"][0]
            self.assertEqual(detail["appIDs"],["ABCD123456.com.acheflow"])
            self.assertEqual([item["/"] for item in detail["components"]], ["/fwd/*", "/server/fwd/*"])
            self.assertFalse((output / "client.acheflow.com").exists())
            config=(output / "Caddyfile").read_text().split("https://app.acheflow.com {",1)[1].split("https://server.acheflow.com",1)[0]
            self.assertIn("handle /fwd/* {",config)
            self.assertIn("rewrite * /forward.html",config)
            self.assertEqual((root / "_redirects").read_text(),"/fwd/* /forward 200\n/server/fwd/* /server-forward 200\n")
            headers=(root / "_headers").read_text()
            self.assertTrue(headers.startswith("/fwd/*\n"))
            self.assertNotIn("\n/*\n",headers)
            self.assertIn("  Cache-Control: no-store",headers)
            self.assertIn("  Referrer-Policy: no-referrer",headers)
            self.assertIn("header @linkResources",config)

    def test_signed_metadata_has_one_handler_for_both_routes_and_no_old_host(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "sites"
            builder.build(output, "ABCD123456", "PREFIX1234")
            root = output / "app.acheflow.com"
            metadata = json.loads((root / ".well-known/apple-app-site-association").read_text())
            detail = metadata["applinks"]["details"][0]
            with (output / "macos/links.entitlements").open("rb") as file:
                entitlement = plistlib.load(file)
            self.assertEqual(detail["appIDs"], ["PREFIX1234.com.acheflow"])
            self.assertEqual([c["/"] for c in detail["components"]], ["/fwd/*", "/server/fwd/*"])
            self.assertEqual(entitlement["com.apple.application-identifier"], detail["appIDs"][0])
            self.assertEqual(entitlement["com.apple.developer.associated-domains"], ["applinks:app.acheflow.com"])
            for recipient, name in (("client", "forward"), ("server", "server-forward")):
                self.assertIn(f'data-recipient="{recipient}"', (root / f"{name}.html").read_text())
            self.assertFalse((output / "server.acheflow.com").exists())
            config = (output / "Caddyfile").read_text()
            self.assertNotIn("https://acheflow.com", config)
            self.assertNotIn("https://server.acheflow.com", config)
            self.assertIn("handle /server/fwd/* {", config)
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
                        {"handler_id": "bad/id"}):
            with tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary) / "sites"
                with self.assertRaises(ValueError):
                    builder.build(output, **options)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
