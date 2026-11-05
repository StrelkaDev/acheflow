"""Build a relocatable, ad-hoc-signed macOS desktop distribution (not notarized)."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile

from runtime_libraries import bundle_libraries

ROOT = Path(__file__).resolve().parents[2]


def verify_version(binary, name, version):
    result = subprocess.run([str(binary), "--version"], capture_output=True, timeout=5, check=True)
    if result.stdout != f"{name} {version}\n".encode():
        raise ValueError(f"{binary}: version must be {name} {version}")


def copy_extensions(source, destination):
    ids = set()
    for manifest_path in sorted(source.glob("*/manifest.json")):
        manifest = json.loads(manifest_path.read_text())
        if manifest["id"] in ids:
            raise ValueError("Duplicate extension ID")
        ids.add(manifest["id"])
        names = ["manifest.json", manifest["module"]]
        browser = manifest.get("authentication", {}).get("browser_module")
        if browser:
            names.append(browser)
        for name in names:
            if Path(name).name != name or "\\" in name:
                raise ValueError("Extension artifact must be a local filename")
            artifact = (manifest_path.parent / name).resolve(strict=True)
            if artifact.parent != manifest_path.parent.resolve():
                raise ValueError("Extension artifact escapes its installation")
            if name == manifest["module"] and not artifact.read_bytes().startswith(b"\0asm\x01\0\0\0"):
                raise ValueError("Invalid WASM module")
            target = destination / manifest_path.parent.name / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(artifact, target)
    if not ids:
        raise ValueError("No extension artifacts found")


def sign(app):
    contents = app / "Contents"
    main_executable = plistlib.loads((contents / "Info.plist").read_bytes())["CFBundleExecutable"]
    for path in sorted((contents / "Frameworks").glob("*.dylib")):
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(path)], check=True)
    for path in sorted((contents / "MacOS").iterdir()):
        if path.is_file() and path.name != main_executable:
            subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(path)], check=True)
    for helper in sorted((contents / "Helpers").glob("*.app")):
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(helper)], check=True)
    subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(app)], check=True)
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)


def build_app(output, recipient, binaries, web, version, module_cache):
    app = output / ("Acheflow.app" if recipient == "client" else "Acheflow Server.app")
    subprocess.run([
        sys.executable, str(Path(__file__).with_name("build-dev-app.py")),
        "--output", str(app), "--recipient", recipient,
        "--bundle-id", "com.acheflow.client" if recipient == "client" else "com.acheflow.server.links",
        "--module-cache", str(module_cache),
        "--ui-bin" if recipient == "client" else "--server-bin",
        str(binaries / ("acheflow-ui" if recipient == "client" else "acheflow-server")),
    ], check=True)
    contents = app / "Contents"
    macos = contents / "MacOS"
    origins = {macos / "acheflow-server": binaries / "acheflow-server"}
    if recipient == "client":
        shutil.copy2(binaries / "acheflow-server", macos / "acheflow-server")
        origins[macos / "acheflow-ui"] = binaries / "acheflow-ui"
        helper = contents / "Helpers/AcheflowLink.app/Contents/MacOS/acheflow-launcher"
        origins[helper] = helper
    else:
        origins[macos / "acheflow-launcher"] = macos / "acheflow-launcher"
    resources = contents / "Resources"
    resources.mkdir(exist_ok=True)
    copy_extensions(ROOT / "extensions", resources / "extensions")
    shutil.copytree(web, resources / "dist")
    minimum = bundle_libraries(contents, origins)
    for plist in [contents / "Info.plist", *contents.glob("Helpers/*.app/Contents/Info.plist")]:
        info = plistlib.loads(plist.read_bytes())
        info.update(CFBundleVersion=version, LSMinimumSystemVersion=minimum)
        plist.write_bytes(plistlib.dumps(info))
    shutil.copy2(ROOT / "crates/acheflow-ui/assets/DejaVu-LICENSE.txt", resources)
    sign(app)
    for name, label in [("acheflow-server", "acheflow-server"), ("acheflow-ui", "Acheflow")]:
        if (macos / name).exists():
            verify_version(macos / name, label, version)
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New distribution directory")
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--skip-build", action="store_true", help="Use already built binaries/web assets")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("This packager requires macOS")
    output = args.output.resolve()
    if output.exists():
        parser.error("Choose a new output directory")
    archive = Path(str(output) + ".zip")
    checksum = Path(str(archive) + ".sha256")
    if archive.exists() or checksum.exists():
        parser.error("Choose an output with no existing archive/checksum")
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    metadata = json.loads(subprocess.check_output([
        cargo, "metadata", "--manifest-path", str(ROOT / "Cargo.toml"),
        "--offline", "--locked", "--no-deps", "--format-version", "1",
    ]))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "acheflow-ui")
    if not args.skip_build:
        subprocess.run([sys.executable, str(ROOT / "extensions/build.py")], check=True)
        profile = ["--release"] if args.profile == "release" else []
        subprocess.run([cargo, "build", "--locked", *profile, "-p", "acheflow-ui", "-p", "acheflow-server", "--bins"], cwd=ROOT, check=True)
        subprocess.run(["trunk", "build", *profile], cwd=ROOT, check=True)
    binaries = Path(metadata["target_directory"]) / args.profile
    for name, label in [("acheflow-ui", "Acheflow"), ("acheflow-server", "acheflow-server")]:
        verify_version(binaries / name, label, version)
    web = ROOT / "dist"
    if not (web / "index.html").is_file() or not list(web.glob("*.wasm")):
        raise ValueError("Build the browser client with trunk before packaging")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="acheflow-swift-") as temporary:
        apps = [build_app(output, recipient, binaries, web, version, Path(temporary))
                for recipient in ("client", "server")]
    shutil.copy2(ROOT / "docs/desktop-installation.md", output / "INSTALL.md")
    inventory = {str(path.relative_to(output)): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(output.rglob("*")) if path.is_file()}
    (output / "package.json").write_text(json.dumps({"version": version, "profile": args.profile,
        "architecture": subprocess.check_output(["/usr/bin/lipo", "-archs", str(apps[0] / "Contents/MacOS/acheflow-ui")], text=True).strip(),
        "signing": "ad-hoc; not notarized", "sha256": inventory}, indent=2) + "\n")
    if archive.exists():
        raise ValueError(f"Archive already exists: {archive}")
    subprocess.run(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(output), str(archive)], check=True)
    checksum.write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n")
    print(archive)


if __name__ == "__main__":
    main()
