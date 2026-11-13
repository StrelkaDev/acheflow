# Acheflow

Acheflow is a studio for content creators, built in Rust. It brings
video sources, scene composition, audio mixing and broadcast outputs into one
workspace, with a shared desktop and browser interface.

The project is developing toward a broader content-creation studio while expanding
its streaming tools. The current foundation is live production: RTMP ingest, GPU
scenes, horizontal and vertical outputs, and extension-based integrations. Future
creator workflows build on that foundation; they are described separately from
implemented features in the [roadmap](docs/roadmap.md).

## What works today

- Receive authenticated RTMP sources at their native resolution.
- Compose scenes with CSS layout, crop, animation, WGSL effects and 3D viewports.
- Add merged chat and extension visual sources, with message CSS and entry/exit animations.
- Mix audio from multiple sources/tracks through scene buses and stereo controls.
- Broadcast horizontal and vertical formats to independent RTMP destinations,
  with CBR/VBR profiles, explicit encoder selection and per-output restart.
- Manage local or remote servers, saved workspaces, previews and traffic status.
- Author client integrations and server logic as sandboxed Rust/WASM extensions.
- Connect provider accounts locally and expose authorized application control via MCP.

The desktop runs enabled, locally authorized provider integrations continuously.
Continuous browser execution and production provider qualification remain unfinished.
See [integration boundaries](docs/client-executor.md) before relying on chat,
audience or automation for a production broadcast.

## Start the desktop studio

For a packaged macOS installation, copy `Acheflow.app` to Applications and open it.
The desktop starts its local server when needed. Settings / Server provides
connection state, ports and Start/Stop/Restart. FFmpeg is required for media work;
the application can open without it. See [installation](docs/desktop-installation.md).

To build and run from the repository root, install Rust with rustup, Python 3,
FFmpeg and the platform's native build dependencies. The checked-in toolchain file
selects Rust and its WASM target. On macOS, native builds also need Xcode Command
Line Tools, OpenSSL development libraries and pkg-config.

```sh
# macOS dependencies, if Homebrew is your package manager:
brew install ffmpeg openssl@3 pkg-config

# Fetch locked dependencies, build extension artifacts, then launch:
cargo fetch --locked
cargo fetch --manifest-path extensions/Cargo.toml --locked
python3 extensions/build.py
cargo run -p acheflow --release --locked
```

A working GPU backend is required for composition. If Cargo is missing from PATH,
load the rustup environment with `source "$HOME/.cargo/env"`.

To build and launch a complete macOS release app with `acheflow://` link
registration, install Trunk and run from the repository root:

```sh
cargo install trunk --version 0.22.1 --locked
python3 native/macos/build-app.py --output target/acheflow-app
open target/acheflow-app/Acheflow.app
```

The script builds the release executable, extensions and browser assets, then
packages `Acheflow.app` with its icon and runtime libraries and registers that
app location with LaunchServices. For a distribution ZIP, use
`python3 native/macos/package.py --output target/acheflow-macos`; it creates a ZIP
and SHA-256 checksum. Choose a new `--output` path for each build; existing outputs
are rejected. Packages are ad-hoc signed and not notarized. See
[desktop installation](docs/desktop-installation.md#build-a-distribution-maintainers)
for packaging checks and installation details.

## Run a server or browser client

The same executable supports a headless server:

```sh
cargo run -p acheflow --release --locked -- --server
```

Default listeners are HTTP `127.0.0.1:21295` and RTMP `127.0.0.1:1935`.
For the browser interface, build its assets and open
[localhost:21295](http://localhost:21295):

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
```

A standalone server serves `dist/`; build the browser assets before using `/`.
For UI development, `trunk serve` uses `http://localhost:8081`. It requires a
separately running server with that exact browser origin allowed.
[Getting started](docs/getting-started.md) covers all launch modes, custom ports,
remote connections, TLS and first broadcast setup.

## One executable, explicit link recipients

`acheflow` opens the desktop; `acheflow --server` runs only the server.
An independent server can add `--native-links` to receive local links on Unix.
The desktop's owned server uses the same executable.

- `acheflow://client/fwd/<extension>/<action>/...` targets the desktop.
- `acheflow://server/fwd/<extension>/<action>/...` targets the local server.

HTTPS forwarding uses `app.acheflow.com`, with `/fwd/...` for the client and
`/server/fwd/...` for the server. OAuth callbacks stay client-only. Hosted fallback
pages and installed handlers must be deployed together; signing and verified
HTTPS app association are separate installation work. See [forwarding](docs/forwarding.md).

## Documentation

Start with the [documentation index](docs/README.md).

| Topic | Guide |
| --- | --- |
| Installation, source builds and launch commands | [Getting started](docs/getting-started.md) |
| Sources, scenes, effects and broadcast outputs | [Live production](docs/live-pipeline.md) |
| Audio tracks, mixing and meters | [Audio](docs/audio-capabilities.md) |
| Provider applications and local account setup | [Provider setup](docs/provider-setup.md) |
| Project structure and trust boundaries | [Architecture](docs/architecture.md) |
| Build, checks and contribution workflow | [Development](docs/development.md) |
| Product direction and current gaps | [Roadmap](docs/roadmap.md) |

Acheflow is under active development. Workspace exports contain RTMP keys and
should be handled as sensitive configuration. Provider credentials stay local to
the client; the logic server does not store them. The media/control HTTP API has
no user-login system and should remain behind an appropriate access boundary.
