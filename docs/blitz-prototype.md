# Dioxus + Blitz UI integration notes

Approved direction: one declarative HTML-like component tree, ordinary CSS and
Rust logic, compiled to native and browser WASM. Dioxus RSX is markup inside Rust;
it is not an independent `.html` template file. Blitz supplies DOM/style/layout
and painting; AnyRender/Vello Hybrid supplies GPU rendering. The application
does not use Dioxus Web's browser-DOM renderer or a WebView.

## Source boundaries

- `crates/acheflow-ui/src/dashboard.rs`: shared components and Rust UI state.
- `live.rs`: shared Live tabs, source/scene editing and stream profiles.
- `audio_mixer.rs`: scene strips, sends, buses, meters and destination routes.
- `studio_mixer.rs`: compact vertical strips for each format's applied scene.
- `slider.rs`: shared bounded audio/effect controls with pointer/keyboard input.
- `layer_editor.rs`: CSS/WGSL inspector, highlighted code and retained drafts.
- `scene_preview.rs`: shared GPU scene editor and offline source patterns.
- `project.rs`: re-export of the shared `acheflow-media-api::workspace` model.
- `media.rs`: shared native/WASM media API client and FFmpeg encoder selection.
- `live_client.rs`: shared native/WASM live-control API, status polling and preview URLs.
- `video_preview.rs`: HTTP JPEG snapshots uploaded into two reusable wgpu textures; avoids unbounded Blitz image URL caching.
- `fps.rs`: FPS editing buffer, validation and common-value controls.
- `workspaces.rs`: workspace creation/switch/import/export and local persistence.
- `dashboard.css`: shared stylesheet, layout, theme, responsive breakpoints.
- `sparks.rs`: custom wgpu texture widget integrated through Blitz/AnyRender.
- `sparks.wgsl`: instanced particle positions, motion, falloff and color on GPU.
- `lib.rs`: font setup and the small native/browser launch differences.
- `index.html`: WASM loader/canvas host only; not a second dashboard.

The design uses a restrained slate control-panel layout: persistent workspace
navigation on desktop, session summary, workspace settings and Live subtabs.
Orange is reserved for actions and the optional GPU effect. No live media data
is fabricated. Workspaces auto-save locally and can be transferred as JSON.

## Integration findings

Both `blitz-dom/custom-widget` and `blitz-paint/custom-widget` must be enabled:
without the latter, the document accepts the custom widget but never paints it.
A widget attached by Dioxus after surface creation also needs first-paint GPU
initialization. `CustomWidgetAttr` is write-once; toggling the effect remounts a
dedicated component that creates a fresh widget instead of reusing the consumed
attribute. Size changes unregister and replace both offscreen texture targets.

The shader uses a 16-byte uniform buffer, no push constants, no compute pass and
no optional GPU features. A Naga test validates its WGSL. The footer reports the
actual adapter backend instead of inferring WebGPU from a successful WASM build.
No WebGL fallback is enabled in this experiment.

Each navigation tab owns a persistent, keyed scroll pane. Inactive panes keep
their layout using `visibility: hidden` and cannot receive pointer events;
switching tabs preserves their independent scroll offsets. The document and
workspace do not scroll, and the header/sidebar remain outside these panes.
Live subtabs also own independent panes. Regression check: scroll Live / Settings
down, visit Scenes and Sources, then return to Settings. Other tabs start at the
top; Settings returns to its previous position. This is shared by native and WASM.
Inactive Live panes include their parent visibility in their active state so
nested visibility rules cannot expose content over another top-level page.

## Current limits

Upstream is pinned to `e42ee1f2577f968e29ea148a75d2d4f0d2cf19cc` and uses
Dioxus alpha and Blitz/winit beta APIs. Dioxus + Blitz is the selected UI stack. This selection does not complete
production qualification. Browser keyboard/accessibility, IME, paste, focus and
broader CSS compatibility need further qualification. Browser automation's
batched text insertion only produced its first character in this run; ordinary
individual key events worked. Do not count this as verified IME/paste support.
Native HTML select controls rendered without their option labels in the tested
Blitz revision; this UI uses shared button groups for finite choices instead.
Some automated mouse dispatches timed out even though the UI changed; the debug
build has not been performance-qualified.
The browser tool's viewport/DPI emulation produced a scale mismatch, so emulated
mobile screenshots are not accepted as responsive-layout verification.

Debug WASM includes large unoptimized engine code and measured about 6 rAF/s with
sparks. The default browser build now uses release WASM, measured at 60 rAF/s in
the same scene. See [performance measurements](blitz-performance.md). Startup
time, memory use and wider hardware performance still require qualification.
RTMP connections and FFmpeg live workers are implemented; see [live pipeline](live-pipeline.md).
The current application does not implement a WASM plugin runtime.
The extension API remains a separate draft shared-contract crate.

## Reproduction

From the workspace root, with `$HOME/.cargo/bin` in PATH:

```sh
cargo run -p acheflow-server --release --locked
```

In a second terminal, launch the native UI or build the browser UI:

```sh
cargo run -p acheflow-ui --release --locked
# Or build browser assets, then open http://localhost:8080:
trunk build
```

See README for checks. The bundled font carries its license under `assets/`.
