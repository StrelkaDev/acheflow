# Spec: output-audio-routing

Objective: each destination chooses a stereo bus independently for every scene.
Persist a scene→bus map with destination gain/mute; an absent scene route selects
Program (ID 1). This extends old workspace JSON with Serde defaults. Validate
bus/source references, prune routes when scenes are deleted and reset affected
routes when a bus is explicitly removed in the UI.

The compositor decodes/collects each track once per active format, renders the
active scene's buses, then feeds every enabled destination its selected PCM mix.
Destination audio remains one AAC stereo stream, 48 kHz/128 kbps. Applied audio
mix/routing edits can change live without restarting video/encoder processes;
resolution, FPS, destination transport and video-codec changes retain their
existing stop-before-change rule. Runtime status identifies the scene, selected
bus and output levels. Invalid applied references fail before changing live state.

Use existing Rust/Tokio broadcast output pipes; keep bounded queues/cancellation.
Owner files: `compositor.rs`, `live.rs`, shared workspace/status contracts and
the output settings UI. Avoid shell commands assembled from user content.

Commands: `cargo test --workspace --locked`; `cargo test -p acheflow-server
--locked -- --ignored --nocapture`; `cargo build -p acheflow-server --release
--locked`; `trunk build`; `cargo fmt --all -- --check`.

Acceptance: real loopback sources with identifiable stereo tones, two outputs
with different buses, absence of unselected tones, live routing changes with
continuous video frames, mute/solo/gain behavior, reconnect and scene switches.
Verify Audio bus selection in each destination and actual applied route in Studio.
Preserve user workspaces; browser test fixtures must be removed after checking.
