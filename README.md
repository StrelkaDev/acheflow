# Acheflow

Rust/Tokio application for RTMP ingest and broadcasting to multiple RTMP outputs.
The selected UI stack is **Dioxus + Blitz + wgpu**. Desktop and browser WASM use
one component tree, one CSS stylesheet, the same Rust state and GPU spark shader.
The previous egui prototype has been removed from the workspace.

## Workspace

- `crates/acheflow-server`: Tokio/Axum HTTP server, static UI assets and `/healthz`.
- `crates/acheflow-ui`: shared Dioxus RSX components, CSS and Rust logic; native
  executable plus browser WASM library. Blitz paints into a GPU canvas.
- `crates/acheflow-extension-api`: draft serializable manifest and panel contract.
- `crates/acheflow-composition`: shared wgpu/Blitz scene rendering, CSS layout/animation, WGSL effects and native frame readback.
- `crates/acheflow-media-api`: shared workspace/scene model, validation, FFmpeg capabilities and live-control types.

Token-authenticated RTMP ingest, FFmpeg decoding/encoding, live scene composition,
RTMP outputs and snapshot previews are implemented. WASM plugin execution and
chat providers are not connected. Workspace configuration is saved locally;
see workspace management below. The shared extension panel renderer is in
`crates/acheflow-ui/src/extension_panel.rs`; no guest modules are loaded.

## Run

Rust is managed by `rust-toolchain.toml`; `Cargo.lock` pins dependencies.
Install FFmpeg on the server (`brew install ffmpeg` on macOS). The server checks
PATH and standard macOS Homebrew locations, or uses `ACHEFLOW_FFMPEG` when set.
FFmpeg runs as a child process; libav libraries are not linked into the GUI.
If Cargo is missing from PATH, run `source "$HOME/.cargo/env"` first.

The server compositor requires a working wgpu adapter (Metal/Vulkan/DX12);
initialization failures are explicit. It runs without a browser or window.

Native window:

```sh
cargo run -p acheflow-ui --release --locked
```

Browser UI, from the workspace root:

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
cargo run -p acheflow-server --release --locked
```

Open http://localhost:8080. Trunk builds optimized release WASM by default into
`dist/`; the server serves that directory. Before building, `/` returns 404.
UI-only development with live rebuild: `trunk serve` on localhost:8081.

The HTTP server binds to `127.0.0.1:8080`; RTMP ingest binds to
`127.0.0.1:1935` (`ACHEFLOW_RTMP_ADDR` overrides the socket). Override its socket address with
`ACHEFLOW_HTTP_ADDR`, or its asset directory with `ACHEFLOW_WEB_ROOT` (default
`dist`, relative to process cwd). Ctrl-C stops HTTP gracefully. `/healthz` reports
HTTP process liveness, not media readiness. Remote WebGPU needs HTTPS; localhost
is suitable for development. No WebGL fallback is enabled in this UI.

## Workspaces and Live

Open Settings in the sidebar for panel appearance and the Sparks background.

Click the workspace name in the sidebar to open workspace settings. On narrow
screens, the sidebar becomes a compact navigation area above the content. Create an
empty workspace, switch between existing workspaces, export the active workspace
as JSON, or add one from a JSON file. Import creates a separate workspace and
never replaces an existing one. Rename a workspace in this panel.

Each workspace owns its expected RTMP sources, scenes/layers, selected Studio
scenes, horizontal/vertical base resolutions and FPS, and per-format RTMP outputs
(URL, key, resolution, quality, bitrate, video codec/encoder and enabled state). The current output
profile uses AAC audio. Video codecs and encoder implementations come from the
server FFmpeg capability API: H.264, and HEVC/AV1/VP9 with Enhanced RTMP on FFmpeg
7+. A destination must support the chosen Enhanced RTMP codec. Scenes inherit the base resolution for their
format; changing it scales source-layer coordinates and dimensions.

Live contains Analytics, Sources, Scenes, Studio and Settings. In Sources add
inputs, copy their URL/token into OBS, and click **Apply sources / listen**. New
tokens are 256-bit random values. Duplicate publishers and invalid tokens are
rejected; applying a rotated token disconnects its old publisher. Source canvas
size controls the decoder output; incoming metadata and frame/audio counters
show the received data.

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
worker counters. The output itself runs at the configured FPS, independently
of the snapshot refresh rate. See [live pipeline](docs/live-pipeline.md).

Browser workspaces auto-save in localStorage for the current origin. Native
workspaces auto-save to `~/.acheflow/workspaces.json`; override this path with
`ACHEFLOW_WORKSPACE_FILE`. Desktop writes use an atomic file replacement and
owner-only permissions on Unix. The UI and workspace model are shared; the
small storage adapter differs by target. There is no server synchronization yet;
use JSON to transfer workspaces between browser and desktop. Concurrent windows
use the most recent local write.

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
liveness still remains healthy. Native UI uses `http://127.0.0.1:8080`, configurable
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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p acheflow-ui --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
cargo test -p acheflow-server --locked -- --ignored --nocapture
trunk build
curl --fail http://localhost:8080/healthz
```

Browser tracing retains warnings/errors without per-frame TRACE timings.
Opt-in local frame diagnostics: open `http://localhost:8080/?perf=1` and read
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

WASM extensions are separate from compiling the UI to WASM. Intended scope is
server logic and GUI panels; see [extension boundary](docs/extensions.md).
The bundled DejaVu font has its license under `crates/acheflow-ui/assets/`.

## Repowise

The local index uses structural documentation, without model calls or API keys.

```sh
repowise status
repowise doctor
repowise update --working-tree
```

Before the first Git commit there is no HEAD/history, so incremental updates may
be limited. Rebuild when necessary:

```sh
repowise init . --yes --no-prose --no-editor-setup --no-agents --no-claude-md --no-hook --no-save-key
```

Project MCP settings live in `.codex/config.toml` and `.mcp.json`. Restart the MCP
session after changing them. Repowise 0.59.0 doctor can still probe the global
Claude MCP entry pointing to Germina rather than this project's override;
validate Acheflow directly with `repowise mcp /Users/achedev/acheflow` or CLI search.
No commit has been created.

Sources: [Blitz](https://github.com/DioxusLabs/blitz),
[WASM examples](https://github.com/DioxusLabs/blitz/blob/main/HOWTO_WASM.md),
[GPU widget example](https://github.com/DioxusLabs/blitz/tree/main/examples/wgpu_texture).
