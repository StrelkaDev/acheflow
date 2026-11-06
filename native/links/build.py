"""Generate isolated forwarding sites and optional matching Apple association metadata."""
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


def build(output, team_id=None, app_prefix=None,
          client_handler_id="com.acheflow.client.links",
          server_handler_id="com.acheflow.server.links"):
    if output.exists():
        raise ValueError("Choose a new output directory")
    if team_id and not re.fullmatch(r"[A-Z0-9]{10}", team_id):
        raise ValueError("Apple Team ID must contain 10 uppercase letters/digits")
    if app_prefix and (not team_id or not re.fullmatch(r"[A-Z0-9]{10}", app_prefix)):
        raise ValueError("App prefix requires a Team ID and 10 uppercase letters/digits")
    for bundle_id in (client_handler_id, server_handler_id):
        if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", bundle_id):
            raise ValueError("Invalid handler bundle ID")
    if client_handler_id == server_handler_id:
        raise ValueError("Client and server need different handler identities")
    source = Path(__file__).resolve().parent
    template = (source / "page.html").read_text()
    blocks = []
    for host, recipient, bundle_id in (("app.acheflow.com", "client", client_handler_id),
                                      ("server.acheflow.com", "server", server_handler_id)):
        root = output / host
        root.mkdir(parents=True)
        shared = host == "app.acheflow.com"
        page = template.replace("{{recipient}}", recipient)
        assets = root / "_acheflow-links" if shared else root
        assets.mkdir(exist_ok=True)
        if shared:
            page = page.replace('href="/page.css"','href="/_acheflow-links/page.css"').replace('src="/entry.mjs"','src="/_acheflow-links/entry.mjs"')
        page_name = "forward.html" if shared else "index.html"
        (root / page_name).write_text(page)
        for asset in ("page.css", "page.mjs", "entry.mjs"):
            shutil.copy2(source / asset, assets / asset)
        # Pages canonicalizes .html requests before serving the asset; proxy to
        # the extensionless URL so the provider's /fwd/... address stays intact.
        pages_target = "forward" if shared else "/"
        (root / "_redirects").write_text(f"/fwd/* /{pages_target.lstrip('/')} 200\n")
        paths = ["/fwd/*", "/forward", "/forward.html", "/_acheflow-links/*"] if shared else ["/*"]
        page_headers = "".join(f"{path}\n" + "".join(f"  {name}: {value}\n" for name,value in SECURITY_HEADERS.items()) + "\n" for path in paths)
        (root / "_headers").write_text(page_headers)
        if team_id:
            application_id = f"{app_prefix or team_id}.{bundle_id}"
            association = {"applinks": {"details": [{
                "appIDs": [application_id],
                "components": [{"/": "/fwd/*", "comment": "Extension forwarding only"}],
            }]}}
            well_known = root / ".well-known"
            well_known.mkdir()
            (well_known / "apple-app-site-association").write_text(json.dumps(association, indent=2) + "\n")
            entitlements = output / "macos"
            entitlements.mkdir(exist_ok=True)
            with (entitlements / f"{recipient}.links.entitlements").open("wb") as file:
                plistlib.dump({
                    "com.apple.application-identifier": application_id,
                    "com.apple.developer.team-identifier": team_id,
                    "com.apple.developer.associated-domains": [f"applinks:{host}"],
                }, file)
        caddy_headers = "\n".join(f'        {name} "{value}"' for name,value in SECURITY_HEADERS.items())
        header_match = "    @linkResources path /fwd/* /forward.html /_acheflow-links/*\n" if shared else ""
        header_target = " @linkResources" if shared else ""
        blocks.append(f"""https://{host} {{
    root * {{$ACHEFLOW_LINK_ROOT}}/{host}
{header_match}    header{header_target} {{
{caddy_headers}
    }}
    @association path /.well-known/apple-app-site-association
    header @association Content-Type application/json
    @links path /fwd/*{'' if shared else ' /'}
    handle @links {{
        rewrite * /{page_name}
        file_server
    }}
    handle {{
        file_server
    }}
}}
""")
    (output / "Caddyfile").write_text("\n".join(blocks))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--apple-team-id")
    parser.add_argument("--apple-app-prefix")
    parser.add_argument("--client-handler-id", default="com.acheflow.client.links")
    parser.add_argument("--server-handler-id", default="com.acheflow.server.links")
    args = parser.parse_args()
    try:
        build(args.output, args.apple_team_id, args.apple_app_prefix,
              args.client_handler_id, args.server_handler_id)
    except ValueError as error:
        parser.error(str(error))
    print(args.output.resolve())
