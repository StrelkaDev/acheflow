# Verification — extensions, interaction, 3D and MCP

Verified on 2026-11-02 on macOS, working tree based on `cec85fc`. No commit or
merge performed. Pre-existing uncommitted media/traffic changes are retained.

## Automated checks

- `cargo test --workspace --locked --offline`: 105 passed. The two local-listener
  tests require execution outside the filesystem/network sandbox; both pass there.
- Final affected UI rerun: 22 unit tests and 7 editor integration tests passed.
- `cargo test -p acheflow-composition --features headless --locked --offline -- --ignored`:
  2 real Metal GPU tests passed, including default viewport equivalence, depth and
  independently transformed secondary viewport content.
- `cargo test -p acheflow-server --locked --offline -- --ignored --test-threads=1`:
  8 real FFmpeg/RTMP/audio/VideoToolbox tests passed. Encoded output includes a
  WASM-normalized chat command changing a 3D object.
- Workspace Clippy with `--all-targets -- -D warnings`, affected UI Clippy after
  final edits, and UI WASM Clippy passed.
- Native workspace build, final native UI build, and final `trunk build` passed.
- Formatting and `git diff --check` passed.

Native UI tests dispatch actual Blitz pointer/key/wheel events: picking, world-axis
movement, dragging outside the view, Escape restoration, wheel-up zoom direction,
and camera-preview isolation. Numeric draft and CSS/WGSL editor tests also pass.
WASM tests cover instruction fuel, host copy costs, memory/import restrictions,
capabilities and bounded template expansion before allocation.

## Browser verification

Verified Live / Scenes in the final web build: separate working view and clean
active-camera preview, hierarchy selection, bottom inspector, world-axis movement
and rotation updating numeric properties, independent orbit and wheel zoom. The
viewport test image is a placeholder, not a connected Twitch/video source. No
browser warning/error logs were observed. Sample guest panel execution and its
counter action were also checked during integration.

Viewport content editors remain mounted while hidden, so mode/object switches
preserve their existing CSS/WGSL draft state. Editor camera and guides remain
transient and are excluded from serialized scene/broadcast output.

## Review and remaining qualification

Repowise refreshed the working-tree graph. Its change-risk report remains
`review_required` for structural complexity/size findings in the larger combined
working tree, including analytics recording, Twitch transport, scene preview,
3D GPU descriptors and existing media fixtures. These are maintainability
reminders, not automatic approval. Focused manual review checked coordinate/math
consistency, bounded guest/transport work, resource lifetime, revision conflicts,
draft retention and preservation of existing media work. Tests above cover the
behavioral boundaries; no additional functional blocker was found.

Live Twitch OAuth/EventSub/outgoing chat verification is pending a public Twitch
application Client ID and user authorization. Provider-shaped fixtures do not prove
that live-account step. YouTube/VK, GLB/glTF import, PBR, lights and authored
animations are subsequent capabilities, not implemented in this iteration.
