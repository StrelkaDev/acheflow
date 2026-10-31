# FFmpeg integration

FFmpeg runs on the Tokio server as a managed executable. Browser WASM and native
GUI use the same capability response and controls. The native Blitz `net` feature
provides its HTTP runtime. Browser requests use the page origin; native requests
use `ACHEFLOW_SERVER_URL` (default `http://127.0.0.1:8080`).

## Discovery

Set `ACHEFLOW_FFMPEG` on the server to an executable path, or install FFmpeg on
PATH. On macOS the server also checks `/opt/homebrew/bin/ffmpeg` and
`/usr/local/bin/ffmpeg`. An explicit override is respected even if it fails;
there is no silent fallback to another FFmpeg build.

At startup and on an explicit refresh the server runs these fixed commands:

```sh
ffmpeg -hide_banner -nostdin -version
ffmpeg -hide_banner -nostdin -encoders
ffmpeg -hide_banner -nostdin -protocols
ffmpeg -hide_banner -nostdin -muxers
```

Each command has a five-second deadline and kill-on-drop; stdin is closed.
Commands are constructed as argument arrays without a shell or user arguments.
Scans are serialized. Failures clear the cached capabilities and return an
explicit error instead of retaining a stale successful list. Missing FFmpeg
does not prevent managing workspaces or serving the GUI.

`GET /api/media/capabilities` reads the cache. `POST` to the same endpoint rescans.
The response includes `available`, `version`, `encoders` (name, codec, media type,
description and experimental flag), `rtmp_output`, `flv_muxer`, `enhanced_rtmp`
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
```

Official references: [FFmpeg CLI options](https://ffmpeg.org/ffmpeg.html),
[encoder documentation](https://ffmpeg.org/ffmpeg-codecs.html), and the
[FFmpeg FLV codec mapping](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/flvenc.c).
