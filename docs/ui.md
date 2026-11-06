# Shared native and browser UI

Acheflow uses one Dioxus RSX component tree, ordinary CSS, Rust state and GPU widgets
on native and browser targets. Blitz supplies DOM/style/layout and painting;
AnyRender/Vello Hybrid supplies GPU rendering. The browser entry HTML hosts the
WASM/WebGPU canvas. It is not a second dashboard or a WebView.

## Owning modules

| Area | Source under `crates/acheflow-ui/src/` |
| --- | --- |
| Navigation and workspace UI | `dashboard.rs`, `workspaces.rs` |
| Server selection and process state | `server_settings.rs`, `server_settings/native.rs` |
| Sources, scenes, output profiles | `live.rs`, `live_client.rs`, `media.rs` |
| Scene/camera manipulation | `spatial_editor.rs`, `spatial_view.rs`, `spatial_viewport.rs` |
| Layer CSS/WGSL and retained drafts | `layer_editor.rs`, `code_editor.rs`, `scene_canvas.rs` |
| Audio strips, sends and meters | `audio_mixer.rs`, `studio_mixer.rs`, `slider.rs` |
| Generic extension panels and local auth | `extension_panel.rs`, `client_authorization.rs` |
| Reusable snapshot textures | `video_preview.rs` |
| Optional background widget | `sparks.rs`, `sparks.wgsl` |
| Shared layout/theme | `dashboard.css` |

The shared model is in `acheflow-media-api`; `project.rs` re-exports it. Native
filesystem/vault/process adapters and browser storage/authorization adapters are
kept separate from the shared visual tree.

## Rendering and input invariants

Both `blitz-dom/custom-widget` and `blitz-paint/custom-widget` are needed for GPU
widgets. A document accepting a widget does not prove that its paint path is enabled.
Dynamically mounted widgets need first-paint GPU initialization. `CustomWidgetAttr`
is consumed once; remount rather than reusing it. Resize unregisters old offscreen
texture targets before replacing them.

Navigation uses persistent keyed scroll panes. Hidden panes keep their scroll
positions and cannot receive pointer input; nested Live visibility also respects
the parent page. Layer IDs preserve code drafts, selection and cursor across layer
selection/reorder. The scene editor camera and guides never enter broadcast output.

Finite choices use shared button groups where native HTML select behavior is
insufficient. Buttons, disclosures, sliders and local account requirements have
explicit keyboard handling alongside pointer handling. Credential inputs are
masked on both targets, including the native text-rendering path.

The actual dependency revisions are pinned in Cargo manifests/lockfiles. The stack
contains prerelease APIs; broader accessibility, IME, paste/focus, DPI and hardware
performance qualification remains necessary. No WebGL fallback is enabled.

## Frame-pacing diagnostics

Use optimized browser WASM for normal work. `Trunk.toml` defaults to release mode;
debug engine code is unsuitable as a performance baseline. Browser tracing keeps
WARN/ERROR and omits per-frame Performance Timeline spans.

```sh
trunk build
cargo run -p acheflow --release --locked -- --server
```

Open `http://localhost:21295/?perf=1` and inspect
`document.documentElement.dataset.acheflowPerf` in developer tools after warm-up.
`perf.js` reports requestAnimationFrame timing; it does not measure GPU timestamps
or guarantee that every callback was presented. Compare the same visible scene,
canvas size and machine without a concurrent build. Separate background-widget
measurements from live media composition. Use ordinary URLs without `?perf=1`
outside diagnostics. No diagnostic data is sent to an external service.

A custom widget requesting frames can trigger broader Blitz scene layout/painting,
so profile the whole redraw path before changing shader detail. Final broadcast
FPS and JPEG snapshot refresh are independent of browser frame pacing.
