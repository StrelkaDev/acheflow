"""Generate shared-domain forwarding pages and optional Apple association metadata."""
import argparse
import json
from pathlib import Path
import plistlib
import re
import shutil

SECURITY_HEADERS = {
    "Content-Security-Policy": "default-src 'none'; script-src 'self'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
    "Referrer-Policy": "no-referrer",
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
}


def build(output, team_id=None, app_prefix=None, handler_id="com.acheflow"):
    if output.exists():
        raise ValueError("Choose a new output directory")
    if team_id and not re.fullmatch(r"[A-Z0-9]{10}", team_id):
        raise ValueError("Apple Team ID must contain 10 uppercase letters/digits")
    if app_prefix and (not team_id or not re.fullmatch(r"[A-Z0-9]{10}", app_prefix)):
        raise ValueError("App prefix requires a Team ID and 10 uppercase letters/digits")
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", handler_id):
        raise ValueError("Invalid handler bundle ID")
    source = Path(__file__).resolve().parent
    template = (source / "page.html").read_text()
    root = output / "app.acheflow.com"
    assets = root / "_acheflow-links"
    assets.mkdir(parents=True)
    for recipient, name in (("client", "forward"), ("server", "server-forward")):
        page = template.replace("{{recipient}}", recipient)
        page = page.replace('href="/page.css"', 'href="/_acheflow-links/page.css"').replace('src="/entry.mjs"', 'src="/_acheflow-links/entry.mjs"')
        (root / f"{name}.html").write_text(page)
    for asset in ("page.css", "page.mjs", "entry.mjs"):
        shutil.copy2(source / asset, assets / asset)
    # Extensionless targets avoid Pages canonicalization of .html URLs.
    (root / "_redirects").write_text("/fwd/* /forward 200\n/server/fwd/* /server-forward 200\n")
    paths = ["/fwd/*", "/server/fwd/*", "/forward", "/forward.html", "/server-forward", "/server-forward.html", "/_acheflow-links/*"]
    (root / "_headers").write_text("".join(f"{path}\n" + "".join(f"  {name}: {value}\n" for name, value in SECURITY_HEADERS.items()) + "\n" for path in paths))
    if team_id:
        application_id = f"{app_prefix or team_id}.{handler_id}"
        association = {"applinks": {"details": [{
            "appIDs": [application_id],
            "components": [{"/": path, "comment": "Extension forwarding only"} for path in ("/fwd/*", "/server/fwd/*")],
        }]}}
        well_known = root / ".well-known"
        well_known.mkdir()
        (well_known / "apple-app-site-association").write_text(json.dumps(association, indent=2) + "\n")
        entitlements = output / "macos"
        entitlements.mkdir()
        with (entitlements / "links.entitlements").open("wb") as file:
            plistlib.dump({
                "com.apple.application-identifier": application_id,
                "com.apple.developer.team-identifier": team_id,
                "com.apple.developer.associated-domains": ["applinks:app.acheflow.com"],
            }, file)
    headers = "\n".join(f'        {name} "{value}"' for name, value in SECURITY_HEADERS.items())
    (output / "Caddyfile").write_text(f"""https://app.acheflow.com {{
    root * {{$ACHEFLOW_LINK_ROOT}}/app.acheflow.com
    @linkResources path {' '.join(paths)}
    header @linkResources {{
{headers}
    }}
    @association path /.well-known/apple-app-site-association
    header @association Content-Type application/json
    handle /fwd/* {{
        rewrite * /forward.html
        file_server
    }}
    handle /server/fwd/* {{
        rewrite * /server-forward.html
        file_server
    }}
    handle {{
        file_server
    }}
}}
""")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--apple-team-id")
    parser.add_argument("--apple-app-prefix")
    parser.add_argument("--handler-id", default="com.acheflow")
    args = parser.parse_args()
    try:
        build(args.output, args.apple_team_id, args.apple_app_prefix, args.handler_id)
    except ValueError as error:
        parser.error(str(error))
    print(args.output.resolve())
