"""Verify an extracted macOS ZIP outside the checkout, with isolated data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def smoke(app, version, working):
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    binary = app / "Contents/MacOS/acheflow"
    assert {p.name for p in binary.parent.iterdir()} == {"acheflow"}
    assert not (app / "Contents/Helpers").exists()
    handler = app / "Contents/Info.plist"
    import plistlib
    info = plistlib.loads(handler.read_bytes())
    assert info["CFBundleURLTypes"][0]["CFBundleURLSchemes"] == ["acheflow"]
    environment = {k: v for k, v in os.environ.items() if not k.startswith(("ACHEFLOW_", "DYLD_"))}
    # Unix-domain sockets have a short platform path limit; extracted package
    # paths intentionally contain spaces and may exceed that limit.
    runtime = tempfile.TemporaryDirectory(prefix="af-link-", dir="/tmp")
    environment.update(PATH="/usr/bin:/bin", ACHEFLOW_FFMPEG="/nonexistent/ffmpeg",
                       ACHEFLOW_DESKTOP_INSTANCE="a" * 32, ACHEFLOW_RUNTIME_DIR=str(Path(runtime.name) / "run"))
    port, rtmp = free_port(), free_port()
    while port == rtmp:
        rtmp = free_port()
    process = subprocess.Popen([str(binary), "--server", "--native-links", "--desktop-managed", "--listen", "127.0.0.1",
        "--port", str(port), "--rtmp-listen", "127.0.0.1", "--rtmp-port", str(rtmp),
        "--config", str(working / "server.json")], cwd=working, env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def get(path):
        with opener.open(f"http://127.0.0.1:{port}{path}", timeout=2) as response:
            return response.read()
    try:
        deadline = time.monotonic() + 20
        while True:
            if process.poll() is not None:
                raise AssertionError(process.stderr.read().decode())
            try:
                info = json.loads(get("/api/server/info"))
                break
            except urllib.error.URLError:
                if time.monotonic() > deadline:
                    raise AssertionError("Installed server never became ready")
                time.sleep(0.1)
        assert info["version"] == version and info["process_id"] == process.pid
        assert info["desktop_attached"] and info["managed_instance"] == "a" * 32
        assert b"Acheflow" in get("/")
        assets = app / "Contents/Resources/dist"
        wasm = next(assets.glob("*.wasm"))
        assert hashlib.sha256(get("/" + wasm.name)).digest() == hashlib.sha256(wasm.read_bytes()).digest()
        state = json.loads(get("/api/control/state"))
        assert any(item["id"] == "sample" for item in state["extensions"]), "Bundled server guest missing"
        process.stdin.close()
        assert process.wait(timeout=12) == 0, "Managed server did not stop gracefully"
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        process.stderr.close()
        runtime.cleanup()
    print(f"PASS {app.name}: version, identity, guest, web/WASM, graceful stop; no developer PATH/FFmpeg")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="acheflow installed package ") as temporary:
        installed = Path(temporary)
        subprocess.run(["/usr/bin/ditto", "-x", "-k", str(args.archive.resolve()), str(installed)], check=True)
        roots = list(installed.iterdir())
        assert len(roots) == 1
        package = roots[0]
        assert {path.name for path in package.iterdir()} == {
            "Acheflow.app", "INSTALL.md", "package.json"
        }, "Distribution includes unexpected build artifacts"
        manifest = json.loads((package / "package.json").read_text())
        for name, checksum in manifest["sha256"].items():
            assert hashlib.sha256((package / name).read_bytes()).hexdigest() == checksum, name
        for name in ["Acheflow.app"]:
            working = installed / (name + " data")
            working.mkdir()
            smoke(package / name, manifest["version"], working)
        print("PASS ZIP inventory and relocated installation with spaces")


if __name__ == "__main__":
    main()
