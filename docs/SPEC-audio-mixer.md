# Spec: audio-mixer

Status: implemented. See [audio capabilities](audio-capabilities.md) for current
limits, controls and verification commands.

Objective: each scene owns reusable stereo mixes that destinations can share.
Shared Rust/Serde contracts persist source+track strips, gain, mute, scene-wide
solo, stereo balance/pan, per-bus sends and master gain/mute. The default bus is
ID 1, Program; it cannot be removed. A legacy scene implicitly routes track zero
of each distinct source represented by a video layer into Program. Additional
discovered tracks start unrouted until explicitly added to the scene mix.

Use fixed-size 20 ms/48 kHz stereo blocks, bounded track/bus counts and finite
integer controls. Gain/send/master: 0–200 percent; pan: -100 left to +100 right.
Pan balances the existing stereo pair without swapping L/R. Sum with fixed gains,
then saturate to i16; do not vary gain with the number of connected inputs.
Expose stereo peak levels and clipping. Silence/mute/master controls, isolated
buses and scene-specific configurations must be observable and testable.

Implement contracts in `acheflow-media-api/src/audio.rs`; PCM rendering and
playout helpers in server `audio_mixer.rs`; UI in `acheflow-ui/src/audio_mixer.rs`.
Follow existing Rust 2024/Dioxus components and panel styling. Add no dependencies.

Commands: `cargo test --workspace --locked`; `cargo clippy --workspace
--all-targets --locked -- -D warnings`; `cargo clippy -p acheflow-ui --lib
--target wasm32-unknown-unknown --locked -- -D warnings`; `trunk build`.

Acceptance: exact PCM gain/pan/mute/solo/master assertions, no cross-bus leakage,
no L/R swapping, clipping counts, legacy import/export and independent scenes.
Add a Live/Mixer view with scene selection, track strips, bus routing and meters.
Users explicitly apply mixer edits to the server through the existing Apply flow.

Always validate bounded/unique IDs and source references. Never invent missing
source data or silently use another bus when an explicit route is invalid.
