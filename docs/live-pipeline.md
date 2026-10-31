# Live pipeline

The workspace model is shared by server and native/WASM GUI in
`acheflow-media-api::workspace`; the GUI's `project.rs` re-exports it. Existing
workspace JSON receives a generated identity and default source canvas when
those fields are absent. Importing a workspace in the GUI creates a separate
identity. Disk/browser catalogs remain local; the server holds one applied
workspace at a time and does not auto-start after a restart.

## User flow

1. In Live / Sources add expected inputs. The source URL is built from the
   connected HTTP server hostname and reported RTMP port. Copy the connection
   token into the publisher's RTMP stream-key field. The token is distinct from
   output destination keys. Apply sources to register them on the server.
2. In Scenes create a scene, add source layers, and set placement/size. Layer
   order is compositing order, last on top. Select a layer and drag its corners to resize, or use exact coordinates.
   Layers remain inside the base canvas. The layer list runs front-to-back;
   Forward/Backward changes stacking order. Source crop insets select a region
   in decode-canvas pixels before it is fitted into the destination rectangle.
   Rotation is not implemented.
3. In Studio choose a horizontal and/or vertical scene and start live. With no
   outputs a format renders preview only. Each enabled output gets an independent
   FFmpeg process with its configured codec, encoder, resolution and bitrate.
4. Apply scene/source edits while running. Drag release applies live placement.
   Changing an active format's base, FPS or outputs is rejected until Stop/Start.
   Stop live leaves registered inputs available; deactivate sources stops both
   composition and incoming publishers.

Sources show actual RTMP connection state, decoded frame count, publisher
metadata dimensions/FPS, audio availability and decoder errors. Studio shows
each output's starting/streaming/failed state and FFmpeg progress frame count.
It also lists each selected scene source as receiving video or waiting for RTMP,
so an active compositor with disconnected inputs is not mistaken for a frozen preview.
Reloading the GUI does not stop a server-side live session.
Preview is a JPEG snapshot refreshed twice a second, not a 2-FPS encoded stream.
Preview audio playback is not implemented; source audio is carried by outputs.
The Studio JPEG widget reuses two wgpu textures per visible preview; the editor
uses the shared GPU scene renderer with bounded per-layer textures. It bypasses Blitz's unbounded URL/image caches; widget
resource registrations are automatically released by Blitz when the node drops.
Inactive tabs do not mount preview widgets or fetch snapshots.

## Layer CSS and GPU effects

The layer inspector exposes one CSS editor instead of separate position/size/crop
fields. `.layer` (or `:layer`) is the styled box. Bare declarations are accepted,
or use a full stylesheet with local `@keyframes`. Every layer is an isolated
Blitz document, so its selectors/keyframe names cannot affect other layers or the
control panel. HTML, external CSS imports/fonts/URLs are rejected; no network
provider is installed in these documents.

```css
.layer {
  left: 24px;
  top: 24px;
  width: 60%;
  height: 280px;
  --crop-left: 10%;
  --crop-top: 0px;
  --crop-right: 0px;
  --crop-bottom: 0px;
  object-fit: contain;
  object-position: 50% 50%;
  border: 4px solid #ff9944;
  border-radius: 24px;
  opacity: 0.9;
  animation: pulse 2s ease-in-out infinite alternate;
}
@keyframes pulse { from { opacity: .5; } to { opacity: 1; } }
```

Crop insets accept non-negative source-canvas pixels or percentages and must
leave a nonempty region. They are evaluated before effects/fitting; CSS
`clip-path` can additionally clip the decorated box. Crop custom properties do
not currently evaluate `calc()`. `object-fit` supports fill/contain/cover/none/
scale-down; object-position is resolved by Stylo, including percentages/lengths.
Use percentage geometry for layouts that adapt to a changed base canvas;
explicit CSS pixels keep their size. Legacy geometry/crop fields remain as the
fallback for old workspace files, while CSS takes precedence when supplied.

Corner handles and dragging still work. They edit left/top/width/height in the last plain `.layer` rule
in place, preserving crop/decoration/keyframes without growing the stylesheet. Handles describe
the layout box before CSS transforms. The front-to-back list controls layer order.
Draft code is validated on blur/Update layer (Load file is also available).
Invalid WGSL stays in the editor with an error; it does not replace valid code.
Apply scene & sources publishes local edits to the server. CSS is limited to
16 KiB/layer. Stylesheet edits rebuild that layer document; keyframes are sampled
against the common scene timeline, but transitions between separate stylesheet
edits are not retained.

The existing six effects now run as wgpu fragment passes: brightness, contrast,
saturation, grayscale, bounded Gaussian blur approximation, opacity. Up to eight
ordered effects per layer are allowed, each independently enabled/removed/moved.
Custom WGSL uses:

```wgsl
fn effect(uv: vec2<f32>, color: vec4<f32>) -> vec4<f32> {
  return vec4<f32>(mix(color.rgb, vec3<f32>(1.0) - color.rgb,
                      params.strength), color.a);
}
```

The host provides `sample_source(uv)`, `params.time` (seconds), `params.strength`
(amount/100), `params.size` (effect texture dimensions), `params.source_size`
(configured source dimensions). Input/output colors use straight RGBA; a final
host pass premultiplies pixels for CSS/Vello blending. Naga validates WGSL before
GPU compilation. Custom resources, extra entry points, loops/storage writes and
oversized functions are rejected. Shader code is capped at 32 KiB/effect.

`acheflow-composition::gpu::GpuScene` is used by the native/WASM canvas widget and
by the server's windowless wgpu + Blitz/Vello renderer. Input textures and the two
processing textures per layer are reused; changed dimensions unregister the old
resources. CSS decoration/animation is rendered by Blitz over the processed video.
Editor source snapshots arrive at 2 Hz but CSS/WGSL animation can redraw between
snapshots; the server renders at configured FPS independently of any open GUI.
Server time/renderer/errors are exposed in format status. GPU readback produces
RGB24 for FFmpeg; zero-copy hardware-encoder integration is not implemented.
The GUI's reduced-resolution snapshots can differ slightly from output sampling.

## Media path

The Tokio RTMP listener uses `rml_rtmp` session/handshake parsing. Application plus
token must match a registered source. Token comparison is constant-time for
equal-length values; new tokens use OS/WebCrypto randomness (32 bytes). Unknown
tokens, playback requests and duplicate active publishers are rejected. Rotating
or removing a source cancels its publisher/decoders. Authenticated streams are
reconstructed as FLV tag sequences, with extended timestamps preserved.

Each input has separate FFmpeg video and audio decoders fed by bounded channels
of eight packets (maximum packet 2 MiB). Video is fitted/padded into its source
canvas as RGB24; audio is stereo 48 kHz signed PCM. Decoders limit input pictures
to 8,294,400 pixels and single allocations to 64 MiB. The compositor keeps only
the latest input image; after two seconds without a frame that layer contributes
no pixels. Uncovered canvas is black.
GPU source uploads are skipped while the decoded image is unchanged; static effect results are reused. CSS and time-dependent shaders still advance.

Each active orientation has a windowless GPU compositor paced at its configured FPS and a
20-ms audio mixer. Visible source audio is included once even when a source has
multiple layers. PCM is averaged across audible sources to avoid sum clipping;
missing audio is silent. Each output consumes the latest composed image at the
configured rate and a bounded PCM stream, and FFmpeg encodes video plus AAC.
Internal PCM sockets bind to ephemeral loopback ports. Output failure/blocked IO
does not stall the compositor or another output. FFmpeg children are cancelled,
killed/reaped on stop/shutdown; kill-on-drop is also enabled.

Current execution limits: 8 sources, 32 layers per scene, 8 outputs, source decode
canvases at most 2,073,600 pixels, composed/output frames at most 8,294,400 pixels.
FPS remains 1–120; throughput depends on scene size, hardware and encoder.
Composition uses 8-bit RGBA GPU textures and RGB24 readback. HDR/color management,
transition effects, recording, outgoing reconnect/backoff and production A/V
latency qualification are not implemented. Chat and WASM guest execution remain
separate work. Hardware encoders can fail at runtime even when listed by FFmpeg;
their failures are shown in Studio.

## API and local access

- `GET /api/live/status`: owner workspace id, RTMP bind address, source and format/output state; no tokens or destination URLs/keys.
- `PUT /api/live/config`: apply a workspace. Layout/source validation is independent of unfinished output drafts. Running encoding profiles cannot be changed.
- `POST /api/live/start`: `{ "workspace": <workspace>, "formats": ["horizontal", "vertical"] }`. Starts only the requested formats; validates enabled outputs for those formats.
- `POST /api/live/stop`: `{ "workspace_id": "..." }`. Idempotent for the matching workspace.
- `DELETE /api/live/config`: same control body; stop and unregister inputs.
- `GET /api/live/preview/horizontal.jpg` and `vertical.jpg`: composed previews.
- `GET /api/live/source/{id}`: input snapshot.

Preview URLs include a workspace id to avoid displaying another workspace's
frames after a switch. Snapshots use `Cache-Control: no-store`. Errors use an
`error` string; stop/deactivate return 409 for stale owner ids. A server with live
formats or connected publishers cannot be silently replaced by another workspace.
The GUI has an explicit stop/deactivate-active-workspace action for this case.
FFmpeg output errors redact the destination URL and key.

Defaults are HTTP `127.0.0.1:8080` and RTMP `127.0.0.1:1935`. Configure
`ACHEFLOW_HTTP_ADDR`/`ACHEFLOW_RTMP_ADDR` for remote access and an appropriate
access boundary: the HTTP control API has no login/tenant authorization. RTMP
tokens authorize publishers but plain RTMP does not encrypt them. New source URLs
may be edited for external hostnames/NAT; editing the URL does not bind another
listener or open a port. Use a release server build for live workloads.

## Verification

The GPU integration test checks actual CSS border/crop/fitting pixels, custom WGSL,
scene-clock animation and bounded texture bindings on a real GPU adapter.

```sh
cargo test --workspace --locked
cargo test -p acheflow-server --locked -- --ignored --nocapture
cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture
```

The opt-in RTMP integration test uses only ephemeral loopback listeners and
synthetic FFmpeg sources. It checks invalid-token rejection, duplicate-publisher
rejection, decoding, actual red/blue pixels after RTMP re-encoding, non-silent
source audio at the receiver, both orientations, live repositioning without
restarting the format, missing-source fallback, reconnect, token revocation,
stale-owner rejection, live crop/opacity/stacking edits through RTMP, JPEG delivery, and one destination surviving failure of
another. No platform credentials or external streaming services are needed.

References: [rml_rtmp sessions](https://docs.rs/rml_rtmp/latest/rml_rtmp/sessions/index.html),
[FFmpeg raw media and progress options](https://ffmpeg.org/ffmpeg.html).
