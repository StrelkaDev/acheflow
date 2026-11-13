# Live production and media pipeline

The workspace model is shared by server and native/WASM GUI in
`acheflow-media-api::workspace`; the GUI's `project.rs` re-exports it. Existing
workspace JSON receives a generated identity and default offline source pattern size when
those fields are absent. Importing a workspace in the GUI creates a separate
identity. Disk/browser catalogs remain local; the server holds one applied
workspace at a time. After a server restart, reapply the saved workspace; media
production does not resume automatically. [Getting started](getting-started.md)
covers application/server launch and the first broadcast.

## User flow

1. In Live / Sources add expected inputs. The source URL is built from the
   connected HTTP server hostname and reported RTMP port. Copy the connection
   token into the publisher's RTMP stream-key field. The token is distinct from
   output destination keys. Apply sources to register them on the server.
2. In Scenes create a scene, add source layers, and set placement/size. Layer
   order is compositing order, last on top. Select a layer and drag its corners
   to resize, or set exact geometry in the CSS inspector.
   Layers remain inside the base canvas. The layer list runs front-to-back;
   Forward/Backward changes stacking order. Source crop insets select a region
   in native input pixels before it is fitted into the destination rectangle.
   These controls edit the flat layer canvas. 3D object/viewports and camera
   transforms are described in [scene controls](controls.md#3d-scenes-and-viewports).
3. In Mixer select a scene and configure source/track sends and stereo buses.
   Track zero of each distinct video-layer source defaults to Program; additional
   tracks need explicit sends. Destination cards omit routing controls; new outputs
   use Program. Existing JSON routes are preserved. See [audio capabilities](audio-capabilities.md).
4. In Studio choose a horizontal and/or vertical scene and start live. With no
   outputs a format renders preview only. Each enabled output gets an independent
   FFmpeg process with its configured codec, encoder, resolution, bitrate and CBR/VBR
   mode. Settings cards expand/collapse and support removal. CBR uses matching
   min/target/max rates plus encoder-specific CBR switches (including x264 HRD
   and VideoToolbox constant bitrate); VBR retains a capped variable rate. Legacy
   JSON without `bitrate_mode` defaults to VBR. The bitrate setting controls video;
   outgoing traffic also includes AAC audio (128 kbps target) and FLV container bytes.
   Each output has a **Keyframe interval (seconds)** setting (1–20, default 2).
   The encoder receives a GOP limit and periodic keyframe requests on the media
   timeline. Older JSON without `keyframe_interval_secs` uses 2 seconds. Changing
   this interval requires Stop/Start. Studio can restart one destination
   independently with **Restart output**.
5. Apply scene/source/mixer/route edits while running. Drag release applies live
   placement; audio-only changes keep the video encoder running. Changing an
   active format's base, FPS, output transport or video profile is rejected until
   Stop/Start.
   Stop live leaves registered inputs available; deactivate sources stops both
   composition and incoming publishers.

Sources show actual RTMP connection state, decoded frame count, publisher
decoded dimensions, publisher-reported FPS, audio availability and decoder errors. Studio shows
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

## Chat and extension visual sources

In **Live / Sources**, choose **+ Chat source**, then add that source as a layer
in a scene. It displays the merged chat supplied by enabled provider extensions.
The source settings select channels, a message limit, display lifetime and entry/
exit durations. The defaults are twenty messages, fifteen seconds of display,
250 ms for entry and 300 ms for exit. Layer geometry controls placement and size.
Apply source/scene changes to update the server's output.

The source CSS editor styles `.chat`, `.chat-message`, `.chat-author`, `.chat-body`
and `.chat-platform`; role classes include `.broadcaster` and `.moderator`.
`.entering` and `.leaving` select animation phases. CSS can use
`--chat-enter-duration` and `--chat-exit-duration` with custom `@keyframes`.
The renderer owns each node's animation delay so existing messages do not restart
when new data arrives. Moderation deletions disappear immediately; the message
limit drops overflow entries, while timed expiry uses the exit animation.

Message text is escaped, and external CSS resources and scripts are disabled.
Editor previews and the output compositor use the same visual-document renderer,
including transparent areas over lower layers. Visual sources have no audio tracks.

Enabled server extensions can publish their declared visual source types. Installed
types appear beside the built-in Chat button, with extension-defined public settings
and a source CSS editor. The sample extension provides an Action counter source.
Disabled/replaced instances cannot keep publishing stale visual output. See the
[extension source contract](extension-contract.md#visual-source-documents).

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

Crop insets accept non-negative native source pixels or percentages and must
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
Invalid code stays in its layer editor with an error; it does not replace valid
code. Layer/effect identities are saved in workspace JSON (older files receive
identities on import). Switching layers or reordering retains each visited
inspector and its draft, native selection and cursor. Two layers of the same
source have independent identities. CSS/WGSL syntax highlighting shares an
ordinary Blitz textarea, with scroll rails shown only on overflowing axes; no
browser-only editor is used.
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
(native decoded source dimensions, or offline pattern dimensions). Input/output colors use straight RGBA; a final
host pass premultiplies pixels for CSS/Vello blending. Naga validates WGSL before
GPU compilation. Custom resources, extra entry points, loops/storage writes and
oversized functions are rejected. Shader code is capped at 32 KiB/effect.

`acheflow-composition::gpu::GpuScene` is used by the native/WASM canvas widget and
by the server's windowless wgpu + Blitz/Vello renderer. Input textures and the two
processing textures per layer are reused; changed dimensions unregister the old
resources. CSS decoration/animation is rendered by Blitz over the processed video.
Editor source snapshots arrive at 2 Hz. Without a fresh decoded frame, only the
scene editor substitutes a cached detailed test-pattern SourceFrame before GPU
processing. Real snapshots carry their original dimensions in `X-Frame-Width`
and `X-Frame-Height` headers, so px crop, object-fit and shader parameters use
native dimensions. JPEG snapshots now retain those dimensions at quality 95;
there is no preview-only downscale or upscale. The saved source `resolution`
only sizes the offline pattern; it never resizes incoming video. Disconnects/stalls restore the pattern; real
frames replace it on the next snapshot. Studio and broadcast missing inputs
remain black/silent. Studio editor overlays are never painted over composition;
errors, empty states and connection status remain outside its image.
CSS/WGSL animation can redraw between
snapshots; the server renders at configured FPS independently of any open GUI.
Server time/renderer/errors are exposed in format status. GPU readback produces
RGB24 for FFmpeg; zero-copy hardware-encoder integration is not implemented.
JPEG compression and the 2 Hz refresh can still differ from the full-rate output.

## Media path

The Tokio RTMP listener uses `rml_rtmp` session/handshake parsing. Application plus
token must match a registered source. Token comparison is constant-time for
equal-length values; new tokens use OS/WebCrypto randomness (32 bytes). Unknown
tokens, playback requests and duplicate active publishers are rejected. Rotating
or removing a source cancels its publisher/decoders. Authenticated streams are
reconstructed as FLV tag sequences, with extended timestamps preserved.

Each input has a video decoder and up to eight independent audio-track FFmpeg
decoders, fed by bounded channels of eight packets (maximum packet 2 MiB).
The RTMP adapter demultiplexes legacy track zero and Enhanced RTMP v2 OneTrack,
ManyTracks and ManyTracksManyCodecs before decoding. Known Enhanced audio
FourCCs are AAC, Opus, MP3, FLAC, AC3 and EAC3; actual decode support depends on
the installed FFmpeg. ModEx and other unknown extensions fail explicitly.
Video is decoded to native-size RGB24 without scaling or padding; each audio
track becomes stereo 48 kHz signed PCM. FFmpeg performs mono duplication/downmix. A named FFmpeg
`showinfo` filter reports one dimension pair per RGB frame through stderr; a
bounded queue of eight descriptors pairs these with the raw stdout pixels.
Checksums are disabled, informational logs are discarded, and malformed frame
descriptors stop the decoder. Implicit FFmpeg autoscaling is disabled, so changed
input dimensions are supported within the same connection. Frame dimensions
come from decoding, not publisher metadata. Decoders limit input pictures
to 8,294,400 pixels, each dimension to 7680, and single FFmpeg allocations to
64 MiB. The compositor keeps only
the latest input image; after two seconds without a frame that layer contributes
no pixels. Uncovered canvas is black. Scene layer geometry and `object-fit`
control scaling/fitting; use `contain` to preserve aspect ratio or `cover` to
fill the layer with cropping. Pixel crop insets are checked against the actual
source at render time, since offline pattern dimensions need not match it.
GPU source uploads are skipped while the decoded image is unchanged; static effect results are reused. CSS and time-dependent shaders still advance.

Each active orientation has a windowless GPU compositor paced at its configured FPS and a
20-ms audio engine. Each scene owns up to eight stereo buses and 64 strips keyed
by source ID and wire track ID. Legacy scenes route track zero of each distinct
video-layer source once to Program (bus 1); additional tracks start unrouted.
Explicit strips override those defaults, including an empty send list.
Gain/send/master controls span 0–200%, stereo balance −100 to +100, with mute and
scene-wide solo. Float sums retain headroom until each destination gain/mute
and final i16 saturation; there is no source-count averaging. Missing PCM is
silent. Timestamped blocks share a per-source clock and bounded 16-block playout
queues; large discontinuities rebase instead of building a backlog. This is not
a claim of qualified end-to-end A/V synchronization.

Every destination stores its scene→bus map (default Program), so two destinations
can share one mix or use isolated mixes. Live / Mixer edits scene controls;
Destination cards omit routing controls; existing JSON routes remain supported. Studio also shows a single row of vertical channel strips with horizontal scrolling under each format preview, following the applied scene during streaming and the selected scene while offline. It includes source levels/gain/mute/solo/balance, bus master controls and destination names. Apply updates
mixers/routes without restarting video; video/transport changes still require
stopping. Status reports source tracks, stereo peak/RMS, bus levels, clipping,
and the actual applied destination scene/bus. UI meters show block peaks at the
status polling rate rather than latched maximum levels. Each output consumes the
latest composed image and its own bounded PCM stream, encoded as one AAC stereo
track, 48 kHz / 128 kbps.
Internal PCM sockets bind to ephemeral loopback ports. Output failure/blocked IO
does not stall the compositor or another output. FFmpeg children are cancelled,
killed/reaped on stop/shutdown; kill-on-drop is also enabled.

Current execution limits: 8 sources, 32 layers per scene, 8 outputs, source decode
frames at most 8,294,400 pixels, offline source patterns at most 2,073,600 pixels,
and composed/output frames at most 8,294,400 pixels.
FPS remains 1–120; throughput depends on scene size, hardware and encoder.
Composition uses 8-bit RGBA GPU textures and RGB24 readback. HDR/color management,
transition effects, recording, outgoing reconnect/backoff and production A/V
latency qualification are not implemented. Chat and WASM guest execution remain
separate work. Hardware encoders can fail at runtime even when listed by FFmpeg;
their failures are shown in Studio.

## API and local access

- `GET /api/live/status`: owner workspace id, RTMP bind address, source and format/output state; no tokens or destination URLs/keys. Sources report cumulative `received_bytes` and rolling `bitrate_kbps`; outputs report their workspace `index`, `sent_bytes` and `bitrate_kbps`. Input bytes count compressed audio/video payload, output bytes count FFmpeg FLV mux bytes. Rates use a two-second rolling window; transport overhead and GUI preview traffic are excluded. Output counters reset on individual restart; input byte counters span publisher reconnects.
- `PUT /api/live/config`: apply a workspace. Layout/source validation is independent of unfinished output drafts. Running encoding profiles cannot be changed.
- `POST /api/live/start`: `{ "workspace": <workspace>, "formats": ["horizontal", "vertical"] }`. Starts only the requested formats; validates enabled outputs for those formats.
- `POST /api/live/output/restart`: `{ "workspace_id": "...", "orientation": "horizontal", "index": 0 }`. Restarts one enabled destination with its applied profile. Index is the original workspace output index, including disabled entries. Owner mismatches return 409; inactive formats or absent/disabled destinations return 422. Other encoders, the compositor and audio mixer continue running.
- `POST /api/live/stop`: `{ "workspace_id": "..." }`. Idempotent for the matching workspace.
- `DELETE /api/live/config`: same control body; stop and unregister inputs.
- `GET /api/live/preview/horizontal.jpg` and `vertical.jpg`: composed previews.
- `GET /api/live/source/{id}`: input snapshot.

Preview URLs include `?workspace=<workspace-id>` to avoid displaying another
workspace's frames after a switch. Snapshots use `Cache-Control: no-store`. Errors use an
`error` string; stop/deactivate return 409 for stale owner ids. A server with live
formats or connected publishers cannot be silently replaced by another workspace.
The GUI has an explicit stop/deactivate-active-workspace action for this case.
FFmpeg output errors redact the destination URL and key.

Defaults are HTTP `127.0.0.1:21295` and RTMP `127.0.0.1:1935`. Configure
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

The opt-in RTMP integration tests use only ephemeral loopback listeners and
synthetic FFmpeg sources. It checks invalid-token rejection, duplicate-publisher
rejection, decoding, actual red/blue pixels after RTMP re-encoding, non-silent
source audio at the receiver, both orientations, live repositioning without
restarting the format, missing-source fallback, reconnect, token revocation,
stale-owner rejection, live crop/opacity/stacking edits through RTMP, JPEG delivery, and one destination surviving failure of
another. Native-size regressions cover landscape/portrait reconnects despite a
different offline pattern size, and resolution changes inside one decoder.
No platform credentials or external streaming services are needed.

References: [rml_rtmp sessions](https://docs.rs/rml_rtmp/latest/rml_rtmp/sessions/index.html),
[FFmpeg raw media and progress options](https://ffmpeg.org/ffmpeg.html),
[showinfo frame metadata](https://ffmpeg.org/ffmpeg-filters.html#showinfo).

Audio acceptance is `real_scene_mixer_multitrack_sources_and_live_destination_routes`:
it sends stereo tones 440/660 and 880/1100 Hz in one RTMP source and
1320/1540 Hz in another, through two independent output encoders and RTMP sinks.
Spectral checks prove selected-tone presence, bus isolation and L/R balance;
live route/mute/scene edits and source reconnect preserve the running compositor.
Exact PCM tests additionally cover gain, solo, master mute and saturation.

UI sliders replace bounded audio/effect amount fields: channel/bus/destination
gain, sends, stereo balance and effect strength/radius. Studio places gain faders
beside the vertical L/R meters. Dragging keeps control outside the narrow rail;
arrow keys change one step, Page Up/Down ten steps, Home/End select the limits,
and the reset button restores the control default. Changes use the existing
local-save / Apply flow. Resolution and bitrate keep precise numeric editors.
