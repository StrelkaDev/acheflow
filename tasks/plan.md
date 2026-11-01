# Audio implementation plan

Status: completed in `884a334` on 2026-11-01. This is the implementation sequence
for reference; results are recorded in [todo.md](todo.md).
Capability map: [audio capabilities](../docs/audio-capabilities.md).

1. Validate/demultiplex Enhanced RTMP audio and independently decode tracks.
2. Add backward-compatible scene mixer/destination route contracts and PCM tests.
3. Route live PCM buses per destination and expose accurate meters/status.
4. Add scene mixer and destination bus controls to the shared native/WASM UI.
5. Verify real distinct-track/source/stereo signals through different RTMP sinks,
   check reconnect/live edits, run workspace/Clippy/build checks and update localhost.

Remaining limitations: monitor playback is not implemented; long-duration
drift and production A/V latency have not been qualified.
