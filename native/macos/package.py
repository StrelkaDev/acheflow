"""Build a relocatable, ad-hoc-signed macOS desktop distribution (not notarized)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

from runtime_libraries import bundle_libraries

ROOT = Path(__file__).resolve().parents[2]


def build_environment():
    environment = os.environ.copy()
    cargo_home = Path(environment.get("CARGO_HOME", Path.home() / ".cargo"))
    environment["PATH"] = str(cargo_home / "bin") + os.pathsep + environment.get("PATH", "")
    return environment


def rust_tool(name):
    tool = shutil.which(name, path=build_environment()["PATH"])
    if not tool:
        raise ValueError(f"Required build tool not found: {name}")
    return tool


def register_links(app):
    registrar = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
    subprocess.run([registrar, "-f", str(app.resolve())], check=True)


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
    subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(app)], check=True)
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)


def app_path(output):
    return output if output.suffix.lower() == ".app" else output / "Acheflow.app"


def build_app(app, binaries, web, version):
    subprocess.run([
        sys.executable, str(Path(__file__).with_name("build-dev-app.py")),
        "--output", str(app), "--bundle-id", "com.acheflow",
        "--binary", str(binaries / "acheflow"),
    ], env=build_environment(), check=True)
    contents = app / "Contents"
    macos = contents / "MacOS"
    origins = {macos / "acheflow": binaries / "acheflow"}
    resources = contents / "Resources"
    resources.mkdir(exist_ok=True)
    copy_extensions(ROOT / "extensions", resources / "extensions")
    shutil.copytree(web, resources / "dist")
    minimum = bundle_libraries(contents, origins)
    for plist in [contents / "Info.plist"]:
        info = plistlib.loads(plist.read_bytes())
        info.update(CFBundleVersion=version, LSMinimumSystemVersion=minimum)
        plist.write_bytes(plistlib.dumps(info))
    shutil.copy2(ROOT / "crates/acheflow-ui/assets/DejaVu-LICENSE.txt", resources)
    sign(app)
    verify_version(macos / "acheflow", "Acheflow", version)
    return app


def main(*, app_only=False, register=False):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="New .app path or distribution directory; a .app path implies --app-only")
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--skip-build", action="store_true", help="Use already built binaries/web assets")
    parser.add_argument("--app-only", action="store_true", default=app_only,
                        help="Build the app without a distribution ZIP or inventory")
    parser.add_argument("--register-links", action=argparse.BooleanOptionalAction, default=register,
                        help="Register the final app with macOS LaunchServices for acheflow:// links")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("This packager requires macOS")
    output = args.output.resolve()
    if output.suffix.lower() == ".app":
        args.app_only = True
    if output.exists():
        parser.error("Output already exists. Choose a new path or move the previous app aside before rebuilding")
    archive = Path(str(output) + ".zip")
    checksum = Path(str(archive) + ".sha256")
    if not args.app_only and (archive.exists() or checksum.exists()):
        parser.error("Choose an output with no existing archive/checksum")
    cargo = rust_tool("cargo")
    metadata = json.loads(subprocess.check_output([
        cargo, "metadata", "--manifest-path", str(ROOT / "Cargo.toml"),
        "--offline", "--locked", "--no-deps", "--format-version", "1",
    ]))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "acheflow-ui")
    if not args.skip_build:
        environment = build_environment()
        subprocess.run([sys.executable, str(ROOT / "extensions/build.py")], env=environment, check=True)
        profile = ["--release"] if args.profile == "release" else []
        subprocess.run([cargo, "build", "--locked", *profile, "-p", "acheflow", "--bin", "acheflow"], cwd=ROOT, env=environment, check=True)
        subprocess.run([rust_tool("trunk"), "build", "--release", str(args.profile == "release").lower()], cwd=ROOT, env=environment, check=True)
    binaries = Path(metadata["target_directory"]) / args.profile
    verify_version(binaries / "acheflow", "Acheflow", version)
    web = ROOT / "dist"
    if not (web / "index.html").is_file() or not list(web.glob("*.wasm")):
        raise ValueError("Build the browser client with trunk before packaging")
    app = app_path(output)
    app.parent.mkdir(parents=True, exist_ok=True)
    apps = [build_app(app, binaries, web, version)]
    if args.app_only:
        if args.register_links:
            register_links(apps[0])
        print(apps[0])
        return
    shutil.copy2(ROOT / "docs/desktop-installation.md", output / "INSTALL.md")
    inventory = {str(path.relative_to(output)): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(output.rglob("*")) if path.is_file()}
    (output / "package.json").write_text(json.dumps({"version": version, "profile": args.profile,
        "architecture": subprocess.check_output(["/usr/bin/lipo", "-archs", str(apps[0] / "Contents/MacOS/acheflow")], text=True).strip(),
        "signing": "ad-hoc; not notarized", "sha256": inventory}, indent=2) + "\n")
    if archive.exists():
        raise ValueError(f"Archive already exists: {archive}")
    subprocess.run(["/usr/bin/ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(output), str(archive)], check=True)
    checksum.write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n")
    if args.register_links:
        register_links(apps[0])
    print(archive)


if __name__ == "__main__":
    main()
