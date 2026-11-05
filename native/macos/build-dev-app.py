"""Build an unsigned client/server URL-handler bundle without registering it."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--recipient", choices=("client", "server"), default="client")
parser.add_argument("--ui-bin", type=Path)
parser.add_argument("--server-bin", type=Path)
parser.add_argument("--bundle-id", help="Base identity; client handler adds .links")
parser.add_argument("--module-cache", type=Path, help="Swift build cache outside the bundle")
args = parser.parse_args()
if sys.platform != "darwin":
    parser.error("This development bundle requires macOS")
if args.output.suffix != ".app" or args.output.exists():
    parser.error("Choose a new .app output directory")
binary = args.ui_bin if args.recipient == "client" else args.server_bin
if not binary or not binary.is_file():
    parser.error("Supply a built --ui-bin for client or --server-bin for server")
if (args.recipient == "client" and args.server_bin) or (args.recipient == "server" and args.ui_bin):
    parser.error("Supply only the binary matching the recipient")
bundle_id = args.bundle_id or f"com.acheflow.{args.recipient}.development"
if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", bundle_id):
    parser.error("Invalid bundle ID")
root = Path(__file__).resolve().parents[2]
cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
metadata = json.loads(subprocess.check_output([
    cargo, "metadata", "--manifest-path", str(root / "Cargo.toml"),
    "--offline", "--locked", "--no-deps", "--format-version", "1",
]))
version = next(package["version"] for package in metadata["packages"] if package["name"] == "acheflow-ui")
contents = args.output / "Contents"
macos = contents / "MacOS"
macos.mkdir(parents=True)
server = args.recipient == "server"
helper = contents if server else contents / "Helpers" / "AcheflowLink.app" / "Contents"
(helper / "MacOS").mkdir(parents=True, exist_ok=True)
cache = args.module_cache or args.output.parent / ".acheflow-swift-modules"
subprocess.run([
    "/usr/bin/xcrun", "swiftc", "-parse-as-library", "-module-cache-path", str(cache),
    str(Path(__file__).with_name("ClientLauncher.swift")),
    "-o", str(helper / "MacOS" / "acheflow-launcher"),
], check=True)
target = macos / ("acheflow-server" if server else "acheflow-ui")
shutil.copy2(binary, target)
os.chmod(target, 0o755)
base_info = {
    "CFBundleIdentifier": bundle_id,
    "CFBundleName": "Acheflow Server" if server else "Acheflow",
    "CFBundleExecutable": "acheflow-launcher" if server else "acheflow-ui",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": "1",
    "CFBundleShortVersionString": version,
    "NSHighResolutionCapable": True,
}
handler_info = {
    **base_info,
    "CFBundleIdentifier": bundle_id if server else bundle_id + ".links",
    "CFBundleName": "Acheflow Server" if server else "Acheflow Client Link",
    "CFBundleExecutable": "acheflow-launcher",
    "LSUIElement": True,
    "AcheflowRecipient": args.recipient,
    "NSUserActivityTypes": ["NSUserActivityTypeBrowsingWeb"],
    "CFBundleURLTypes": [{
        "CFBundleURLName": f"com.acheflow.{args.recipient}.forwarding",
        "CFBundleTypeRole": "Viewer",
        "CFBundleURLSchemes": [f"acheflow-{args.recipient}"],
    }],
}
with (contents / "Info.plist").open("wb") as file:
    plistlib.dump(handler_info if server else base_info, file)
if not server:
    with (helper / "Info.plist").open("wb") as file:
        plistlib.dump(handler_info, file)
print(args.output.resolve())
