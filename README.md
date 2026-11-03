# Acheflow

Rust/Tokio application for RTMP ingest and broadcasting to multiple RTMP outputs.
The selected UI stack is **Dioxus + Blitz + wgpu**. Desktop and browser WASM use
one component tree, one CSS stylesheet, the same Rust state and GPU spark shader.
The previous egui prototype has been removed from the workspace.

## Workspace

- `crates/acheflow-server`: Tokio/Axum HTTP server, static UI assets and `/healthz`.
- `crates/acheflow-ui`: shared Dioxus RSX components, CSS and Rust logic; native
  executable plus browser WASM library. Blitz paints into a GPU canvas.
- `crates/acheflow-extension-api`: guest events, audience/chat contracts and host-rendered panels.
- `crates/acheflow-control`: bounded interaction/analytics services and metered WASM execution.
- `crates/acheflow-composition`: shared wgpu/Blitz scene rendering, CSS layout/animation, WGSL effects and native frame readback.
- `crates/acheflow-media-api`: shared workspace/scene model, validation, FFmpeg capabilities and live-control types.

Token-authenticated RTMP ingest, native-resolution video decoding, multitrack
audio, scene mixing, per-destination audio routing, GPU scene composition,
RTMP outputs and snapshot previews are implemented. Server WASM extensions,
Twitch interaction, audience metrics, 3D viewport scenes and authenticated local
MCP control are implemented. Workspace configuration is saved locally;
see workspace management below. The shared extension panel renderer is in
`crates/acheflow-ui/src/extension_panel.rs`; server guests execute separately in wasmi.

## Documentation

- [Live pipeline](docs/live-pipeline.md): source setup, CSS/WGSL scenes, media flow, API and limits.
- [Audio capabilities](docs/audio-capabilities.md): track discovery, scene buses, output routing and verification.
- [FFmpeg integration](docs/ffmpeg.md): discovery, codecs and hardware encoder selection.
- [UI integration notes](docs/blitz-prototype.md) and [measured browser performance](docs/blitz-performance.md).
- [Extensions and WASM ABI](docs/extensions.md): installation, capabilities and guest lifecycle.
- [Interaction, 3D and MCP](docs/controls.md): setup, workflows, API and verification qualifications.

## Run

Rust is managed by `rust-toolchain.toml`; `Cargo.lock` pins dependencies.
Install FFmpeg on the server (`brew install ffmpeg` on macOS). The server checks
PATH and standard macOS Homebrew locations, or uses `ACHEFLOW_FFMPEG` when set.
FFmpeg runs as a child process; libav libraries are not linked into the GUI.
If Cargo is missing from PATH, run `source "$HOME/.cargo/env"` first.

The server compositor requires a working wgpu adapter (Metal/Vulkan/DX12);
initialization failures are explicit. It runs without a browser or window.

Start the server for either UI target, from the workspace root:

```sh
python3 extensions/build.py
cargo run -p acheflow-server --release --locked
```

Native window, in another terminal:

```sh
cargo run -p acheflow-ui --release --locked
```

Browser UI, from another terminal in the workspace root:

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
```

Open http://localhost:21295. Trunk builds optimized release WASM by default into
`dist/`; the server serves that directory. Before building, `/` returns 404.
UI-only development with live rebuild: `trunk serve` on localhost:8081.

The HTTP server binds to `127.0.0.1:21295`; RTMP ingest binds to
`127.0.0.1:1935`. Use `--listen` for the HTTP host/IP and `--port` for its port;
`--listen 127.0.0.1:21296` also accepts a socket address. An explicit `--port`
wins over the port in `--listen`. IPv6 sockets use `[::1]:21295`.

```sh
cargo run -p acheflow-server -- --listen 0.0.0.0 --port 21295
cargo run -p acheflow-server -- --rtmp-listen 0.0.0.0 --rtmp-port 1935
cargo run -p acheflow-server -- --http-cert /certs/fullchain.pem --http-key /certs/key.pem \
  --rtmp-cert /certs/fullchain.pem --rtmp-key /certs/key.pem
```

Configure the HTTP and RTMP host/port and HTTPS/RTMPS certificate chains and
private keys through launch options, environment variables or the server JSON
configuration. These restart-only settings are not editable in the UI.
Certificate and key values are PEM file paths on the server. Each listener uses TLS
when both paths are set, and plain HTTP/RTMP when both are empty. TLS replaces
plain transport on that listener's port. Certificate/key pairs are validated
before startup and before saving. Use unencrypted PEM private keys (encrypted keys are rejected without prompting).
TLS uses OpenSSL (a development installation
is required when compiling the server, e.g. `brew install openssl@3 pkg-config`
on macOS).

The server reads `acheflow-server.json` in the process working
directory. Use `--config /path/server.json` or `ACHEFLOW_CONFIG` to select another
file. Changes require a server restart. Priority: CLI arguments > environment overrides > saved
settings > defaults. Existing `ACHEFLOW_HTTP_ADDR` and `ACHEFLOW_RTMP_ADDR`
socket overrides remain supported; TLS environment variables are
`ACHEFLOW_HTTP_CERT`, `ACHEFLOW_HTTP_KEY`, `ACHEFLOW_RTMP_CERT`, `ACHEFLOW_RTMP_KEY`.
Remove the corresponding overrides on restart to use values from the JSON file.
`--help` lists all launch options.

Use `ACHEFLOW_WEB_ROOT` to change the asset directory (default `dist`, relative
to process cwd). Ctrl-C stops listeners and media workers gracefully. `/healthz`
reports process liveness, not media readiness. Remote WebGPU needs HTTPS;
localhost is suitable for development. No WebGL fallback is enabled in this UI.
For HTTPS, trust the issuing certificate in the operating system/browser.
`ACHEFLOW_SERVER_URL` supplies the native UI's initial address for legacy
workspaces; newly created workspaces save their own server address.

For a browser UI hosted at a different origin, allow its exact origin on the
target server, e.g. `ACHEFLOW_UI_ORIGINS=https://studio.example,http://localhost:8081`.
The comma-separated list enables CORS only for the media/live API; server settings
are excluded. Same-origin browser access and the native UI need no CORS setting.
An HTTPS page must connect to an HTTPS server to avoid mixed-content blocking.

## Workspaces and Live

Open Settings in the sidebar for application information, panel appearance
and the Sparks background.

Click the workspace name in the sidebar to open workspace settings. On narrow
screens, the sidebar becomes a compact navigation area above the content. Create an
empty workspace, switch between existing workspaces, export the active workspace
as JSON, or add one from a JSON file. Import creates a separate workspace and
never replaces an existing one. Rename a workspace in this panel.

The native client and a standalone static web client ask for a name and HTTP(S)
server address when creating a workspace. This address is shown in the workspace
panel and preserved in local storage and exported files. Old files without an
address retain the browser origin or native startup default. Creation does not
require the server to be online; connection failures are shown in Live and retried.

When opening the web interface hosted by a running Acheflow server, the UI detects
its same-origin `/api/live/status` at startup and stays attached to that server.
Workspace creation shows the server address in a read-only field and asks for a name.
Imported server addresses do not redirect
this server-hosted UI to another backend. A static web client without a same-origin
live API uses per-workspace addresses, like the native client.

API requests, FFmpeg capability discovery and all previews use the selected
workspace's server, including an optional reverse-proxy path. RTMP source creation
waits for server discovery, then combines the selected hostname with the active
RTMP port and TLS mode returned by `/api/live/status`. Listener bind addresses
such as `0.0.0.0` are not used as public ingest hostnames.

Each workspace owns its enabled extensions and their public configuration,
expected RTMP sources, scenes/layers, selected Studio
scenes, horizontal/vertical base resolutions and FPS, and per-format RTMP outputs
(URL, key, resolution, quality, CBR/VBR bitrate mode, video codec/encoder and enabled state). The current output
profile uses AAC audio. Video codecs and encoder implementations come from the
server FFmpeg capability API: H.264, and HEVC/AV1/VP9 with Enhanced RTMP on FFmpeg
7+. A destination must support the chosen Enhanced RTMP codec. Scenes inherit the base resolution for their
format; changing it scales source-layer coordinates and dimensions.

Live contains Analytics, Sources, Scenes, Mixer, Studio and Settings. In Sources add
inputs, copy their URL/token into OBS, and click **Apply sources / listen**. New
tokens are 256-bit random values. Duplicate publishers and invalid tokens are
rejected; applying a rotated token disconnects its old publisher. **Offline preview
width/height** only size the scene editor's test pattern. Incoming video keeps
its decoded resolution; resize/crop/fit it in the scene layer. Sources show
decoded dimensions, publisher metadata and frame/audio counters.

Add sources to a scene, select a layer and drag its corners to resize. The layer
inspector has one CSS editor for position, size, crop (`--crop-left/top/right/bottom`
in px/%), `object-fit`/`object-position`, borders, rounding, opacity and local
`@keyframes`. Dragging updates a geometry rule in the same CSS. Change stacking
with Forward/Backward. Add/reorder/disable GPU effects or write a WGSL fragment
function in the shader editor. CSS and shader code are stored in workspace JSON.
CSS and WGSL fields have syntax highlighting and draggable scrollbars that appear
only on overflowing axes. Unapplied/invalid text, selection and cursor stay with their layer
when selecting or reordering layers; validation still runs on blur/Update layer.
The scene editor uses a detailed test image for a source without fresh video,
including the same GPU crop, fitting, CSS and effect chain. Live video replaces it
automatically. Studio shows only composition pixels, with diagnostics outside
its image; test images are confined to the scene editor.
Apply inspector edits to update live output. Select
scenes in Studio and click **Start live**. Both selected formats run; enabled
outputs publish RTMP, and a format with no outputs runs preview only. Use
**Apply scene & sources** for edits while running; dragging applies on release.
Resolution/FPS/encoding changes require Stop/Start. **Stop live** retains inputs
for editing; **Deactivate sources** stops inputs as well. Studio shows output
errors and real server-composited JPEG previews at 2 Hz. Analytics uses actual
worker counters and rolling bitrate charts for receiving/sending, with totals and
individual sources/destinations over the last two minutes. Incoming rates count
compressed media payload; outgoing rates count encoded FLV bytes, including audio.
Network overhead and GUI preview requests are excluded.

Destination cards in Settings expand/collapse independently of their saved profile;
opening one closes the previous card, and Remove deletes that destination.
CBR fixes the video target; VBR varies with image complexity up to the configured
limit. Old workspaces without `bitrate_mode` retain the previous capped VBR behavior.
Encoder rate control follows [FFmpeg codec options](https://ffmpeg.org/ffmpeg-codecs.html).
**Restart output** in Studio restarts only that destination using its applied
profile; the compositor, input publishers and other destinations keep running.
JPEG snapshots retain native dimensions at quality 95 (no forced reduction to
480 pixels). Snapshot refresh remains 2 Hz. The output itself runs at the configured FPS, independently
of the snapshot refresh rate. See [live pipeline](docs/live-pipeline.md).

In **Live / Mixer**, select a scene, configure source/track strips and sends, and
add or adjust stereo buses. Track zero of each distinct video-layer source starts
on Program; additional tracks start unrouted. Gain, sends and bus/destination
gain span 0–200%; balance spans −100 to +100. Solo applies across the scene.
Destination cards use Program audio by default and omit routing controls. Existing
destination bus/gain/mute settings remain compatible in workspace JSON. Apply
mixer edits to update running audio without restarting video. Each output encodes
one AAC stereo track.

Studio also has a horizontally scrollable row of vertical channel strips below
each format preview, with L/R meters, gain faders, mute/solo/balance and bus
masters. It follows the applied scene while live and the selected scene while
offline. Edits save locally; **Apply mixer** sends them to the server. Meters
report server levels; there is no local monitor playback.

Browser workspaces auto-save in localStorage for the current origin. Native
workspaces auto-save to `~/.acheflow/workspaces.json`; override this path with
`ACHEFLOW_WORKSPACE_FILE`. Desktop writes use an atomic file replacement and
owner-only permissions on Unix. The UI and workspace model are shared; the
small storage adapter differs by target. The same catalog now saves the global
background-animation preference. Connected control updates merge public extension
settings, capability policy and 3D state into the local workspace; conflicts preserve
local drafts until an explicit choice. Use JSON to transfer workspaces between browser
and desktop. Concurrent windows use the most recent local catalog write.

Client integrations are discovered locally (bundled in browser builds); server
snapshots list only server logic. Their public workspace settings live separately
in `client_extensions` and are excluded from Apply/Start. The Extensions page has Client/Server tabs: local integrations belong to Client,
while server logic, permissions and connected executors belong to Server. Each tab
identifies its current client/workspace or selected server endpoint.

The server's active workspace and public policy remain in memory: after a server
restart, apply the saved client workspace again. Server listener/TLS path settings
persist separately in the server JSON file. See [settings persistence and restore](docs/settings-persistence.md)
for storage locations, Apply behavior, conflict recovery and verified restart limits.

JSON exports include RTMP keys. Import rejects unsupported versions, malformed
configuration, missing references, layers outside the canvas, unsupported output
profiles/encoder identifiers and files larger than 2 MiB. Export reports incomplete configuration
instead of producing an invalid workspace. Local autosave also retains drafts
while fields are being edited. Storage errors are shown in workspace settings; a broken
catalog is not silently overwritten.

## FFmpeg discovery and frame rate

Live / Settings displays the server FFmpeg version, encoder inventory and detected
RTMP/FLV support. Each output selects a video codec, Software (CPU) or Hardware,
and a concrete FFmpeg encoder. Studio reports the encoder used by each output.
Encoder selection is saved in the workspace; older
JSON files without the optional `encoder` field import as automatic. A missing
encoder on another server is shown explicitly and is never silently replaced.
The inventory describes encoders compiled into FFmpeg, not a successful hardware
initialization test. Choosing a codec does not start a stream; Studio starts
the selected outputs explicitly.

VideoToolbox hardware selection disables software fallback. On macOS, H.264 and
HEVC can use `h264_videotoolbox` and `hevc_videotoolbox`. Software uses encoders
such as `libx264` and `libx265`; an unavailable explicit encoder is an error.

`GET /api/media/capabilities` returns the cached startup scan. The Refresh
encoders action issues `POST` to that endpoint to rescan. Missing FFmpeg and scan
failures are explicit responses (`available: false`, with an error); HTTP
liveness still remains healthy. Native UI uses `http://127.0.0.1:21295`, configurable
with `ACHEFLOW_SERVER_URL`. Browser WASM uses its own page origin; it never runs
FFmpeg on the client. Start the server for either UI target.

FPS accepts integer values 1–120, independently for horizontal and vertical.
Type a value and press Enter or leave the field to apply it, or use a common-FPS
button. Partial edits do not overwrite the workspace. Invalid/empty edits restore
the applied value and show an error; Escape cancels the edit. This fixes the old
case where the field displayed e.g. 600 while the saved configuration kept 60.

See [FFmpeg integration](docs/ffmpeg.md) for scope and verification.

## Checks

```sh
python3 extensions/build.py
cargo fmt --manifest-path extensions/Cargo.toml --all -- --check
cargo clippy --manifest-path extensions/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path extensions/Cargo.toml --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p acheflow-ui --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
cargo test -p acheflow-server --locked -- --ignored --nocapture
cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture
trunk build
curl --fail http://localhost:21295/healthz
```

Browser tracing retains warnings/errors without per-frame TRACE timings.
Opt-in local frame diagnostics: open `http://localhost:21295/?perf=1` and read
`document.documentElement.dataset.acheflowPerf` in developer tools. Nothing is
sent externally. See [performance measurements](docs/blitz-performance.md).

## Architecture

```text
RTMP publisher -> token/auth (rml_rtmp) -> FLV tags -> FFmpeg RGB/PCM decoders
                                                    |
                       shared scene geometry -> wgpu + Blitz compositor / PCM audio mixer
                                                    |
                           per-format frames -> independent FFmpeg encoders
                                                    -> RTMP destinations
                                                    -> JPEG snapshots -> native/WASM UI
```

FFmpeg decodes inputs at their original resolution, then performs output encoding,
output scaling and RTMP publication. Source `resolution` only sizes the offline
editor pattern. The shared GPU compositor fits native input images into scene
layers and reads back RGB24. Each scene owns an audio mixer with stereo buses,
source/track strips, gain, mute, solo, balance and sends. Each destination chooses
a scene bus, encoded as AAC stereo 48 kHz / 128 kbps. Multiple RTMP sources and
Enhanced RTMP v2 audio tracks are independently decoded. Mixer and route edits
apply live; use Live / Mixer and Live / Settings / Audio routing. Inputs have
bounded packet queues and outputs have independent process/IO lifetimes, so a
failed destination does not stop another output. Missing sources become black
and silent, and can reconnect while the composition keeps running. Outgoing
automatic reconnect and HDR/color management are future work.
The HTTP control API is local by default and has no user-login system.

UI markup is HTML-like Dioxus RSX inside Rust; CSS is a separate ordinary file.
This does not require a duplicate browser-DOM interface or a WebView. The engine
is pinned to Blitz revision `e42ee1f2577f968e29ea148a75d2d4f0d2cf19cc`.
Blitz/Dioxus currently bring beta/alpha APIs: adopting the stack does not imply
all browser behaviors are qualified. Known input, accessibility, DPI and
performance qualifications are recorded in [integration notes](docs/blitz-prototype.md).

WASM extensions are separate from compiling the UI to WASM. Server logic runs in
wasmi and panels are host-rendered. Providers are independent Rust guest crates;
server/UI code stays provider-neutral. See the [architecture contract](docs/extension-contract.md)
and [extension development guide](extensions/README.md).
The bundled DejaVu font has its license under `crates/acheflow-ui/assets/`.

## Repowise

The local index uses structural documentation, without model calls or API keys.
Run commands from this repository root:

```sh
repowise status
repowise doctor
repowise update --working-tree --no-agents
```

Initialize a checkout that has no index with:

```sh
repowise init . --yes --no-prose --no-editor-setup --no-agents --no-claude-md --no-hook --no-save-key
```

Project MCP settings live in `.codex/config.toml` and `.mcp.json`. Restart the MCP
session after changing them. If a tool reports stale files, refresh the index
before relying on its cached context; check the owning source for current behavior.

Sources: [Blitz](https://github.com/DioxusLabs/blitz),
[WASM examples](https://github.com/DioxusLabs/blitz/blob/main/HOWTO_WASM.md),
[GPU widget example](https://github.com/DioxusLabs/blitz/tree/main/examples/wgpu_texture).
