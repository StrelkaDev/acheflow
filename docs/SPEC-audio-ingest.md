# Spec: audio-ingest

Status: implemented. See [audio capabilities](audio-capabilities.md) for current
limits, controls and verification commands.

Objective: preserve distinct incoming audio tracks across RTMP sources and within
Enhanced RTMP v2 audio messages. Source ID and wire track ID form a stable key.

Use the existing Rust 2024/Tokio/rml_rtmp 0.8/FFmpeg subprocess stack. A bounded,
checked packet demultiplexer routes each audio track to its own FFmpeg decoder;
`0:a:0` then selects the sole track of that decoder, rather than losing siblings.
Support ordinary FLV audio, Enhanced Audio OneTrack, ManyTracks and
ManyTracksManyCodecs for the codecs understood by the installed FFmpeg.
Malformed headers, unsupported extensions/codecs and a ninth concurrent audio
track on one source produce a visible input error. Disconnect cancels decoders.

Decoded mixer input is 48 kHz interleaved stereo i16. FFmpeg performs its standard
mono duplication/downmix. Keep track identity, codec, decoded layout, activity,
peak and decode failure status; disconnected/stalled meters must become zero.
Retain presentation timestamps on decoded fixed-size audio blocks to align tracks
of the same source without letting late audio grow an unbounded backlog.

Source: server `audio_packet.rs`, `audio_input.rs` and `ingest.rs`, with shared
`acheflow-media-api/src/audio.rs` status contracts. Tests stay beside the
parser/decoder code and in `audio_pipeline_tests.rs`.
Use typed structs and explicit bounds, e.g. `TrackKey { source_id, track_id }`.
No new dependencies; no shell interpolation or logging stream tokens.

Commands: `cargo test -p acheflow-server --locked`; `cargo test -p acheflow-server
--locked -- --ignored --nocapture`; `cargo clippy --workspace --all-targets
--locked -- -D warnings`; `cargo fmt --all -- --check`.

Acceptance: byte-level malformed/batched packet tests, multiple real decoded
tones from one source, distinct L/R content, reconnect cleanup and bounded queues.

Protocol references:
- https://github.com/veovera/enhanced-rtmp/blob/main/docs/enhanced/enhanced-rtmp-v2.md
- https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavformat/flvdec.c

Always validate untrusted lengths before slicing/allocation. Never silently
discard an extra track. Compatibility is proven with actual FFmpeg and loopback
RTMP, not inferred from a compiled codec list.
