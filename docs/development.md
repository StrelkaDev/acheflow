# Development

Use the repository's pinned Rust toolchain, host lockfile and separate guest
lockfile. [Getting started](getting-started.md) covers prerequisites and launch
modes; [architecture](architecture.md) maps the packages.

## Build and run

```sh
cargo fetch --locked
cargo fetch --manifest-path extensions/Cargo.toml --locked
python3 extensions/build.py
cargo build -p acheflow --release --locked
cargo run -p acheflow --release --locked
cargo run -p acheflow --release --locked -- --server
```

The final two commands launch the desktop or the standalone server. Choose one
when using the default ports. The desktop starts its managed server as needed.
For browser UI assets, install the pinned Trunk version and build:

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
```

Trunk defaults to optimized WASM. `trunk serve` uses localhost:8081 and needs a
separate backend allowing that origin. Native development entrypoints remain
available. Installations use the unified `acheflow` binary. For macOS camera
and microphone verification, [build and launch the app bundle](desktop-installation.md#build-a-distribution-maintainers)
so macOS can prompt for device permissions.

## Standard checks

On a fresh checkout and after guest/SDK changes, build guests before host builds
or tests. Keep source, manifests and generated local WASM artifacts matched.
These build outputs, including the domain test fixture, are ignored by Git and
must not be committed. CI must run the guest build before host checks; macOS
packaging runs it automatically unless explicitly reusing an existing build.
Do not substitute native guest code for real-WASM integration tests.

```sh
python3 extensions/build.py
cargo fmt --manifest-path extensions/Cargo.toml --all -- --check
cargo clippy --manifest-path extensions/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path extensions/Cargo.toml --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p acheflow-ui --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
node --test native/links/page.test.mjs
node --test native/links/callback.test.mjs native/links/relay.test.mjs crates/acheflow-ui/src/client_authorization/browser_auth.test.mjs
node --test crates/acheflow-ui/startup.test.mjs
python3 -m unittest discover -s native/links -p 'test_*.py'
python3 -m unittest discover -s native/macos -p 'test_*.py'
trunk build
```

Real-media/GPU tests are opt-in and need FFmpeg, a GPU adapter and loopback sockets:

```sh
cargo test -p acheflow-server --locked -- --ignored --nocapture
cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture
```

To exercise the optional SRT ingest, select an FFmpeg binary with SRT support
and run the encrypted loopback receiver test.

```sh
ACHEFLOW_FFMPEG="<ffmpeg-with-srt>" cargo test -p acheflow-server --lib --locked real_srt_ingest -- --ignored --nocapture
```

Some native credential tests use the OS vault and should run with isolated fixture
records. A sandbox blocking local listeners cannot validate process/IPC tests.
The production-server HTTPS integration can also be run directly:

```sh
cargo test -p acheflow-server --test shared_https --locked
```

It needs OpenSSL and local listeners. It creates an ephemeral test CA without
installing system trust, then checks cookie pairing, device grants, revocation and
restart recovery with two real fixture WASM collectors. It does not replace
physical-device or live-provider authorization checks.

### Sustained A/V timing

Run the loopback capture with FFmpeg, libx264 and a headless GPU adapter. Then pass
the temporary directory printed by the test to the analyzer, which also uses
ffprobe.

```sh
cargo test -p acheflow-server --lib --locked sustained_rtmp_av_markers -- --ignored --nocapture
python3 crates/acheflow-server/tests/measure_av_timing.py "<capture-directory>" \
  --markers-csv "<capture-directory>/markers.csv" > "<capture-directory>/report.json"
```

The fixture starts with 120 seconds of 320×180 at 30 fps. A second 1080×1920
compositor and libx264 output add GPU and CPU work. Both outputs default to
30 fps. Set `ACHEFLOW_AV_VERTICAL_FPS=25` on the capture command to test a
different output cadence. The analyzer uses each capture's configured rate
for its video packet-gap budget. During the loaded phase, the
publisher inserts a 1.25-second PTS jump on both tracks, disconnects and
reconnects. The fixture also independently restarts the vertical output while
recording the horizontal output continuously. The run takes about four minutes.

The analyzer checks the source reference, pairs flash and tone onsets by PTS,
and reports signed offset (positive means audio later), drift, missing pairs,
packet gaps and media duration. The provisional steady-state budget is 80 ms
for the 95th percentile of absolute offset, 150 ms peak, and 20 ms/min baseline
drift. It also checks at least 95% matched pulses, gaps of at most 4.1 seconds
between matched pulses, and packet gaps of at most three video frame periods
(plus 1 ms for timestamp rounding) or 65 ms of audio. Transition windows are
reported separately, including the times of the first and last matched pulses.
The capture and analysis can succeed while the reported output exceeds the
budget. Keep captures and reports outside the repository.

The capture also writes `marker-trace.csv`. It records flash and tone onsets at
decode, composition, mixing, and the raw video encoder feed. `observed_s` is
monotonic time since tracing started. `frame_or_block_s` is the image render time
or the scheduled PCM block start at the mixer. `pts_48khz` is the source PTS
through the mixer and the output slot PTS at the encoder. Each `epoch` row gives
the scheduled origin for that encoder attempt. Use the trace to locate the stage
where the onsets diverge and the FLV report to measure the receiver's offset.

The shorter `live_manual_av_trim` probe changes a running scene from 200 ms of
audio delay to 200 ms of source video delay, checks the applied track and
server-load status, and saves the resulting RTMP stream.

`audio_minus_video_ms` in the CSV is the sound onset PTS minus the matching
flash PTS. Plot it against `video_pts_s` separately for each capture. A flat
line above zero means the audio is consistently late. A sloping series means
the offset is changing. A step after a reconnect or timestamp jump isolates
a change in synchronization at that transition. Use `p95_absolute_offset_ms`
to compare typical worst-case offsets across runs and `peak_absolute_offset_ms`
to spot isolated outliers. Frame counters and bitrate cannot measure this offset.

### Mixed-output preview load at 30 and 60 FPS

Use an optimized server build, FFmpeg with `libx264` and SRT, and a headless
GPU adapter. The probe drives a real horizontal RTMP destination (1280×720)
and a vertical SRT destination (720×1280) on loopback, both at 30 FPS and then
both at 60 FPS. A synthetic 320×180 / 30 FPS RTMP source provides flash and
audio markers. Set `ACHEFLOW_PREVIEW_PROBE_ENCODER=h264_videotoolbox` with a
compatible FFmpeg build to exercise the hardware encoder.

```sh
cargo test -p acheflow-server --release --lib --locked mixed_output_preview_load_at_30_and_60_fps -- --ignored --nocapture
python3 crates/acheflow-server/tests/measure_av_timing.py --preview-load "<capture-directory>" \
  > "<capture-directory>/report.json"
```

Each rate runs three eight-second windows: outputs alone, two viewers per
format and snapshot type (JPEG and lossless PNG), then two viewers per
format for encoded preview. Snapshot requests invoke the server handlers
directly, with a target of 10 requests per second per viewer. Response times
exclude browser decoding and network transfer. The JSON separates render
cadence and render p95 from snapshot responses, output frame repeats, source
reconnect and the RTMP restart while SRT stays connected.

The A/V report measures marker offsets and packet gaps in the real receiver
captures. It summarizes each capture as a whole, including transitions in the
continuous streams. Use [the sustained probe](#sustained-av-timing) for
longer steady-state windows and drift qualification. Keep the generated
captures and reports outside the repository.

On an Apple M5 Max with FFmpeg 9.0.2, both formats held their 30/60 FPS
targets in every window with `libx264` and `h264_videotoolbox`. Render p95
across the sampled windows was 1.48–3.39 ms. Raw snapshot viewers received
about 10 images/s with a response p95 of at most 14 ms. Encoded preview
delivered 8.1–9.6 images/s including encoder warm-up. The RTMP restart took
0.71–0.72 s while the SRT output continued. Across the original RTMP and
continuous SRT captures, p95 absolute A/V offset ranged from 12.5 to 80 ms.
These short measurements use a simple synthetic scene and loopback transport.

### Output-track profiling

To collect separate latency samples for the output's video and audio tracks,
start the server with `ACHEFLOW_ENCODER_PROFILE=1` and start the outputs.
This enables FFmpeg's verbose timestamp diagnostics for the output processes.
Studio reports a rolling p95 of FFmpeg's per-packet pipeline time for each
track. When FFmpeg reports an isolated encode stage, Studio also shows its p95
over the samples that contain that stage. FFmpeg can combine stages or leave
them out below its reporting threshold. The timing ends before network writes
and is independent of the receiver's A/V offset. Use profiling during diagnosis
because FFmpeg writes per-packet traces.

```sh
ACHEFLOW_ENCODER_PROFILE=1 ./target/release/acheflow --server
```

Run affected checks after changes; reuse successful evidence only for unchanged
code/dependencies/environment. Documentation-only edits need link, command, content
and privacy checks rather than an unrelated media-suite rerun.

## Packaging and UI checks

Use [desktop packaging](desktop-installation.md) for the macOS archive builder and
relocated-installation smoke test. Its ad-hoc signing is distinct from public
Developer ID/notarization and HTTPS app association.

Check actual native/browser rendering when changing UI behavior. Verify state and
numeric values with focused DOM/tests, and capture final visuals when layout is
relevant. Do not treat a compiled WASM file as proof of a working GPU or live provider.
[UI integration](ui.md) covers rendering/input and frame-pacing diagnostics.

## Repository context

Follow [AGENTS.md](../AGENTS.md) and the [extension contract](extension-contract.md).
Use Repowise for code relationships, risk and architectural context; verify changed
behavior in the owning source. Refresh uncommitted context when needed:

```sh
repowise status
repowise doctor
repowise update --working-tree --no-agents
```

An unindexed checkout can initialize structural context without editor/hook changes:

```sh
repowise init . --yes --no-prose --no-editor-setup --no-agents --no-claude-md --no-hook --no-save-key
```

Project MCP settings are in `.codex/config.toml` and `.mcp.json`. Start clients from
the checkout; machine-specific overrides belong in user configuration.
Do not commit, merge or rewrite commits without an explicit request.

## Documentation and verification data

The root README introduces the studio and launch paths. `docs/README.md` indexes
current guides; `docs/roadmap.md` is the single summary of unfinished product work.
Keep architecture decisions in `docs/decisions/`, clearly marking superseded choices.
Remove completed implementation plans and session logs when current guides replace
them; Git history retains their development context.

Use obvious placeholders in examples. Do not put real application/client/account/
channel/project IDs, user names, email addresses, callback payloads, secrets,
workspace/session IDs, process IDs or machine-specific paths from live checks into
documentation. Do not turn test-account configuration into defaults. Protocol
versions, schema field names, documented limits and public project/source links
are legitimate technical references. Keep private verification data outside tracked
docs and remove it from any artifact prepared for sharing. Scan current Markdown
and validate local links before finishing a documentation change.
