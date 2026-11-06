"""Build an unsigned unified Acheflow bundle without registering it."""
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
parser.add_argument("--binary", type=Path, required=True)
parser.add_argument("--bundle-id", help="Main application and URL-handler identity")
args = parser.parse_args()
if sys.platform != "darwin":
    parser.error("This development bundle requires macOS")
if args.output.suffix != ".app" or args.output.exists():
    parser.error("Choose a new .app output directory")
binary = args.binary
if not binary.is_file():
    parser.error("Supply a built --binary")
bundle_id = args.bundle_id or "com.acheflow.development"
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
target = macos / "acheflow"
shutil.copy2(binary, target)
os.chmod(target, 0o755)
base_info = {
    "CFBundleIdentifier": bundle_id,
    "CFBundleName": "Acheflow",
    "CFBundleExecutable": "acheflow",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": "1",
    "CFBundleShortVersionString": version,
    "NSHighResolutionCapable": True,
}
base_info.update({
    "LSUIElement": True,
    "NSUserActivityTypes": ["NSUserActivityTypeBrowsingWeb"],
    "CFBundleURLTypes": [{
        "CFBundleURLName": "com.acheflow.forwarding",
        "CFBundleTypeRole": "Viewer",
        "CFBundleURLSchemes": ["acheflow"],
    }],
})
with (contents / "Info.plist").open("wb") as file:
    plistlib.dump(base_info, file)
print(args.output.resolve())
