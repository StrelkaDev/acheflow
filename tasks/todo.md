# Audio tasks

- [x] audio-ingest: checked multitrack parser, independent decoders/status and tests.
- [x] audio-mixer: scene contracts, validation, legacy defaults and exact PCM tests.
- [x] output-audio-routing: per-destination PCM, live routes and applied meters.
- [x] Shared UI: Live/Mixer, scene buses and destination routing.
- [x] Verification: real multitrack/multi-source/stereo RTMP and live changes.
- [x] Checks, Repowise refresh, local server/UI rebuild and browser verification.

Verified 2026-11-01 on this Mac with FFmpeg 9.0.2:
- workspace tests: 62 passed; dedicated server integration run: 8 passed.
- Native workspace and WASM Clippy, formatting, release server and Trunk builds.
- Browser: scene mixer, bus add/name/mute/remove, source controls and destination
  bus selection. Temporary bus/output removed; original scene routing retained.
- No monitor playback or long-duration A/V drift qualification is claimed.

Studio follow-up: compact vertical channel strips, horizontal scrolling and gain
faders beside L/R meters. Shared sliders also cover sends, balance, destination
gain and effect amounts. UI tests include pointer/keyboard input, drag-release
focus, shared values and preventing L/R meter wrapping. UI regression run: 17 passed.
