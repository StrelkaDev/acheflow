# FFmpeg integration

FFmpeg runs on the Tokio server as a managed executable. Browser WASM and native
GUI use the same capability response and controls. The native Blitz `net` feature
provides its HTTP runtime. Requests follow the selected workspace server; a
server-hosted browser client keeps its hosting server. `ACHEFLOW_SERVER_URL` supplies
the native legacy-workspace startup default (`http://127.0.0.1:21295`).

## Discovery

Set `ACHEFLOW_FFMPEG` on the server to an executable path, or install FFmpeg on
PATH. On macOS the server also checks `/opt/homebrew/bin/ffmpeg` and
`/usr/local/bin/ffmpeg`. An explicit override is respected even if it fails;
there is no silent fallback to another FFmpeg build.

At startup and on an explicit refresh the server runs these fixed commands:

```sh
ffmpeg -hide_banner -nostdin -version
ffmpeg -hide_banner -nostdin -encoders
ffmpeg -hide_banner -nostdin -h encoder=ENCODER_NAME
ffmpeg -hide_banner -nostdin -protocols
ffmpeg -hide_banner -nostdin -muxers
```

Each command has a five-second deadline and kill-on-drop; stdin is closed.
Encoder help is read for each detected video encoder. Its hardware/hybrid
capabilities and supported hardware devices determine the `hardware` flag.
Commands are constructed as argument arrays without a shell or user arguments.
Scans are serialized. Failures clear the cached capabilities and return an
explicit error instead of retaining a stale successful list. Missing FFmpeg
does not prevent managing workspaces or serving the GUI.

`GET /api/media/capabilities` reads the cache. `POST` to the same endpoint rescans.
The response includes `available`, `version`, `encoders` (name, codec, media type,
description, experimental and hardware flags), `rtmp_output`, `flv_muxer`, `enhanced_rtmp`
and `error`. GET does not launch a child process.

The complete inventory includes video, audio and subtitle encoders. The current
output UI offers non-experimental encoders for H.264 and, conservatively on
FFmpeg 7+, HEVC/AV1/VP9, provided the RTMP output protocol and FLV muxer exist.
Other codecs remain visible in the inventory but are not offered as this
application's RTMP video profiles. AAC remains the audio profile.

Compiled hardware encoders are not evidence of a working hardware device,
driver, supported pixel format or supported resolution. Discovery does not run
hardware initialization tests. Encoder names and codec families are stored
separately; the optional `encoder: null` means automatic selection for the worker (software implementation preferred when present). A saved selection missing on the connected server remains in the
workspace and is visibly marked unavailable. Import does not require the same
FFmpeg installation as the exporting machine.

Each output offers Software (CPU) and Hardware controls followed by matching
encoder implementations. Choosing either type saves a concrete encoder name in
the existing `encoder` field. There is no separate mode that can disagree with
that name. The server and UI use the same resolver; explicit choices never fall
back to another implementation. Automatic legacy configurations prefer software.
Output status reports the chosen `encoder` and its `hardware` flag, including
while an output starts or fails; Studio displays them beside progress.

VideoToolbox outputs use `-allow_sw 0 -require_sw 0 -realtime 1`, requiring hardware
instead of silently using Apple's software encoder. Media Foundation outputs
use `-hw_encoding 1`. Other hardware backends retain their native device and
driver requirements; discovery alone does not validate those. GPU composition,
raw RGB readback, scaling and software AAC are independent of video encoder type.

Live ingest, scene rendering, audio mixing and output workers are documented in
[live pipeline](live-pipeline.md). Choosing a codec or refreshing the inventory
does not launch a broadcast; Studio starts the chosen outputs explicitly.

## Verification

Parser tests cover encoder aliases/families, legends, malformed rows, media types
and experimental flags. Capability tests cover transport/profile filtering.
Workspace tests cover encoder roundtrip, legacy files without `encoder`, and
independent horizontal/vertical FPS. FPS tests cover partial edits, empty/invalid
input, overflow and the accepted range.

The installed-executable test is opt-in so a machine without FFmpeg can run unit
tests. Run it on the integration host:

```sh
cargo test -p acheflow-server installed_ffmpeg_reports_video_audio_and_rtmp --locked -- --ignored --nocapture
cargo test -p acheflow-server installed_hardware_and_software_encoders_produce_real_frames --locked -- --ignored --nocapture
```

Official references: [FFmpeg CLI options](https://ffmpeg.org/ffmpeg.html),
[encoder documentation](https://ffmpeg.org/ffmpeg-codecs.html), and the
[FFmpeg FLV codec mapping](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/flvenc.c).
Hardware requirements follow the upstream [VideoToolbox encoder](https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavcodec/videotoolboxenc.c)
and [Media Foundation encoder](https://ffmpeg.org/doxygen/9.0/mfenc_8c.html).
