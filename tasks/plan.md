# Audio implementation plan

Approved map: `docs/audio-capabilities.md`; scene ownership clarified by user.
No intermediate approval gates requested by the user.

1. Validate/demultiplex Enhanced RTMP audio and independently decode tracks.
2. Add backward-compatible scene mixer/destination route contracts and PCM tests.
3. Route live PCM buses per destination and expose accurate meters/status.
4. Add scene mixer and destination bus controls to the shared native/WASM UI.
5. Verify real distinct-track/source/stereo signals through different RTMP sinks,
   check reconnect/live edits, run workspace/Clippy/build checks and update localhost.

Preserve existing changes. The user authorized one consolidated local commit and merge into main on
2026-11-01. Remote publication requires a separate request.
