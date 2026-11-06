# Getting started

Acheflow has one production executable. Run it as a desktop studio, a headless
server, or a local URI dispatcher. The browser interface connects to an Acheflow
server and cannot launch native processes itself.

## Prerequisites

For an installed macOS package, use [desktop installation](desktop-installation.md).
Developer tools are not needed to run that package. FFmpeg is a runtime dependency
for ingest/encoding; a usable GPU backend is required for composition.

For a source build, install rustup, Python 3, FFmpeg, the platform compiler/linker
and native development libraries. `rust-toolchain.toml` selects the Rust version,
Clippy, rustfmt and `wasm32-unknown-unknown`. `Cargo.lock` pins host dependencies;
`extensions/Cargo.lock` pins the separate guest workspace.

On macOS:

```sh
xcode-select --install
brew install ffmpeg openssl@3 pkg-config
```

Use the system's normal package management if these prerequisites are already
installed. The macOS package workflow is exercised locally; other platforms still
need their own packaging and native activation validation.

Run the following commands from the repository root. If necessary, first run
`source "$HOME/.cargo/env"` so Cargo, rustup and installed Rust tools are on PATH.

```sh
python3 extensions/build.py
cargo build -p acheflow --release --locked
```

The guest builder operates offline with locked dependencies. A fresh checkout may
need to fetch them first:

```sh
cargo fetch --locked
cargo fetch --manifest-path extensions/Cargo.toml --locked
```

## Desktop with a local server

```sh
cargo run -p acheflow --release --locked
# Or, after building:
./target/release/acheflow
```

The desktop starts its configured local server if the selected local workspace
needs one. Settings / Server shows readiness, endpoint, ports, logs and ownership.
Default ports are HTTP 21295 and RTMP 1935. Choose unused ports there if needed.
Stop/Restart controls only this client's child. A compatible independently started
server remains external. A workspace targeting a remote server stays remote.

The optional start-with-client preference also starts a local server for remote
workspaces. Keep-running-on-exit is disabled by default; quitting normally stops
the owned child. See [local server lifecycle](local-server.md).

## Independent server

```sh
./target/release/acheflow --server
```

This runs without a desktop window. Default listeners bind to loopback:
`http://127.0.0.1:21295` and `rtmp://127.0.0.1:1935`. For explicit configuration:

```sh
./target/release/acheflow --server \
  --listen 127.0.0.1 --port 21300 \
  --rtmp-listen 127.0.0.1 --rtmp-port 1940 \
  --config ./server-data/acheflow-server.json
```

Ctrl-C stops the server. `--server --help` lists server options; ordinary `--help`
and `--version` do not initialize a GUI or receiver. On Unix, add `--native-links`
for same-user server URI delivery. Use separate private `ACHEFLOW_RUNTIME_DIR`
values for independent link-receiving installations.

Listener precedence is CLI > environment > saved JSON > defaults. The default
settings file is `acheflow-server.json` in the working directory. `ACHEFLOW_CONFIG`
selects another file. HTTPS/RTMPS use server-local PEM certificate/key paths:

```sh
./target/release/acheflow --server \
  --http-cert /path/to/fullchain.pem --http-key /path/to/private-key.pem \
  --rtmp-cert /path/to/fullchain.pem --rtmp-key /path/to/private-key.pem
```

Use unencrypted PEM private keys. Each certificate needs its matching key; TLS
replaces plain transport on the configured listener. Listener changes need restart.

## Browser client

Build assets, then run a server as above:

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
```

Open [localhost:21295](http://localhost:21295), or the actual configured HTTP(S)
endpoint. Trunk builds optimized WASM into `dist/`; a source server's root page is
unavailable until assets exist. `ACHEFLOW_WEB_ROOT` overrides their directory.
The browser needs WebGPU; localhost or trusted HTTPS is required. No WebGL fallback
is enabled. A UI served by a server remains attached to that server.

For live UI rebuilds, start a separate server with the development origin allowed:

```sh
ACHEFLOW_UI_ORIGINS=http://localhost:8081 \
  ./target/release/acheflow --server
# In another terminal:
trunk serve
```

Open [localhost:8081](http://localhost:8081). If using a different hostname/port,
allow that exact origin. Browser and native catalogs are separate.

## Remote server

Set an explicit server listener for remote access, such as `--listen 0.0.0.0`;
use `--rtmp-listen 0.0.0.0` only when remote publishers need it. Connect the native
workspace to the server's actual HTTP(S) endpoint in Settings / Server.
The media/control HTTP API has no user-login system: provide a trusted network or
an authenticated proxy rather than exposing it directly. Configure TLS as needed.
A browser on another origin also needs `ACHEFLOW_UI_ORIGINS` with its exact origin;
an HTTPS browser page requires an HTTPS server to avoid mixed-content blocking.
MCP has a separate authenticated loopback listener; media CORS does not grant MCP access.

## First broadcast

1. Create/select a workspace and connect to its server.
2. Add an input in Live / Sources, then apply sources. Copy the generated RTMP
   address/token into a publisher such as OBS.
3. Create a scene and add the source. Adjust layout/crop/effects; apply the scene.
4. Set the scene mix in Live / Mixer. Track zero starts on Program; additional
   tracks need explicit routing.
5. Configure destinations, resolution, FPS, bitrate mode and an available encoder
   in Live / Settings. Streaming keys are stored in the workspace.
6. Select horizontal/vertical scenes in Studio and start the desired outputs.
   Check actual status/errors. JPEG previews refresh separately from encoded FPS.

For provider accounts, enable the client extension and follow
[provider setup](provider-setup.md). Successful local OAuth does not automatically
start the continuous executor. For detailed production controls and limits, use
[live production](live-pipeline.md), [audio](audio-capabilities.md) and
[client execution](client-executor.md).
