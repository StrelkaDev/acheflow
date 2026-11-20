"""Create a host-selectable extension ZIP from built, manifest-declared artifacts."""
import argparse
import json
from pathlib import Path
import zipfile


def package(directory: Path, output: Path):
    directory = directory.resolve(strict=True)
    manifest_path = directory / "manifest.json"
    if manifest_path.stat().st_size > 65536:
        raise ValueError("Manifest exceeds 64 KiB")
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("parts_version") != 1 or not manifest.get("parts"):
        raise ValueError("Rebuild the extension with parts_version 1")
    names = {"manifest.json"}
    modules = set()
    for part in manifest["parts"].values():
        if part is None:
            continue
        modules.add(part["module"])
        names.add(part["module"])
        browser = (part.get("authentication") or {}).get("browser_module")
        if browser:
            names.add(browser)
    artifacts = {}
    for name in sorted(names):
        if Path(name).name != name or "\\" in name or ".." in name:
            raise ValueError("Artifacts must be local filenames")
        path = directory / name
        if path.is_symlink() or path.resolve(strict=True).parent != directory:
            raise ValueError("Artifact escapes the installation")
        if not path.is_file() or path.stat().st_size > 4 * 1024 * 1024:
            raise ValueError("Invalid or oversized artifact")
        data = path.read_bytes()
        if name in modules and not data.startswith(b"\0asm\x01\0\0\0"):
            raise ValueError("Build the WASM guest before packaging")
        artifacts[name] = data
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in artifacts.items():
            archive.writestr(name, data)
    if output.stat().st_size > 8 * 1024 * 1024:
        output.unlink()
        raise ValueError("Package exceeds 8 MiB")
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(package(args.directory, args.output))
