# Audio capability map

Implemented as of 2026-11-01. This map describes the current audio path; the
module specifications below retain its design and acceptance criteria.

## Current scope

- Audio can arrive from multiple RTMP sources and as multiple independent
  audio tracks inside a single compatible RTMP input.
- Each output selects one stereo mixer bus. Multiple independent audio tracks
  inside one output are not implemented.
- Mixer configurations belong to scenes. Destinations store a bus selection for
  each scene, so shared program/clean feeds do not require duplicate mixers.
- The transport adapter demultiplexes legacy track zero and Enhanced RTMP v2
  OneTrack, ManyTracks and ManyTracksManyCodecs. Each track has its own FFmpeg
  decoder; unsupported packet types/codecs produce an explicit input error.
  Actual codec availability depends on the installed FFmpeg and producer.

## Implementation

- `audio_packet.rs` preserves wire track IDs. `audio_input.rs` decodes up to
  eight tracks per source to timestamped 48 kHz stereo i16 blocks. `0:a:0`
  selects the sole track of each independent decoder. FFmpeg converts incoming
  channel layouts to stereo; a track ID is distinct from its L/R channels.
- `acheflow-media-api/src/audio.rs` persists scene strips, buses and destination
  scene→bus maps. Each scene has 1–8 buses and at most 64 explicit strips.
  Program (ID 1) is required. Legacy defaults route track zero of each distinct
  video-layer source once; additional tracks start unrouted. Explicit strips,
  including an empty send list, override the defaults.
- `audio_mixer.rs` renders 20 ms blocks with fixed gains and scene-wide solo;
  source count does not alter gain. Float sums retain headroom until destination
  gain/mute and final i16 saturation. Missing input is silent. Timestamped playout
  queues are bounded to 16 blocks; large discontinuities rebase the clock.
- `compositor.rs` starts one audio engine per active format and independent
  output workers. Each destination receives its selected scene bus, encoded by
  `output.rs` as one AAC stereo track at 48 kHz / 128 kbps.

Server files above live in `crates/acheflow-server/src/`. Shared contracts are in
`crates/acheflow-media-api/src/`. See [live pipeline](live-pipeline.md) for queue,
decoder and execution limits.

## Capability boundaries

| Module id | Responsibility | Depends on |
| --- | --- | --- |
| [audio-ingest](SPEC-audio-ingest.md) | Discover stable source/track identities, retain track metadata, decode each incoming track independently, expose arrival and level information, report unsupported input formats. | — |
| [audio-mixer](SPEC-audio-mixer.md) | Per-scene input strips keyed by source and track; gain, mute, solo and balance; stereo buses and sends; bus gain/mute and meters; bounded PCM processing and explicit clipping. | audio-ingest |
| [output-audio-routing](SPEC-output-audio-routing.md) | Persist each destination's bus selection per scene, feed that bus to its AAC encoder, show the applied route, allow live mixer/routing changes without restarting video encoding. | audio-mixer |

Dependency order: audio-ingest → audio-mixer → output-audio-routing.

Workspace validation rejects duplicate strip/bus IDs and missing source/bus
references. A configured track may be absent or disconnected and then contributes
silence. Legacy imports receive defaults for scene mixers and output routes.

## Controls and apply behavior

Live / Mixer selects a scene and exposes source/track controls, per-bus sends,
bus creation/name/master controls and meters. Gain/send/master/destination controls
span 0–200%; stereo balance spans −100 to +100. Program cannot be removed;
removing another bus resets affected destination routes to Program.

Live / Settings / Audio routing chooses a bus for each destination and scene,
plus destination gain/mute. Studio places a scrollable row of vertical strips
under each format preview, following the applied scene during streaming and the
selected scene offline. It shows source L/R levels, gain/mute/solo/balance,
bus masters and destination names; sends are configured in Live / Mixer.

Edits save locally and reach the server through Apply mixer/routes. Audio-only
edits do not restart video encoders; transport/video-profile changes still require
Stop/Start. Source meters become zero after 500 ms without decoded PCM. UI meters
show block peaks at status polling time, not latched maxima. Meter movement is
not local audio playback; monitoring is not implemented.

## Verification required across the complete path

- Produce independently identifiable tones on different tracks and distinct
  left/right channels, using multiple sources and one multitrack input.
- Check decoded track identities and their content, not just audio presence.
- Prove gain, mute, solo, pan and bus-master behavior against PCM samples.
- Route different buses to two simultaneous local RTMP sinks and verify each
  sink contains the intended tones and excludes the other bus's tones.
- Change gain, mute and output routing live; verify the applied route and that
  the video encoder and frame progression continue without a restart.
- Exercise disconnect/reconnect, missing tracks, silence, clipping, source/bus
  removal, legacy imports and workspace export/import.
- Check bounded queues and audio timing during a sustained run. A test that
  merely finds nonzero samples is insufficient evidence for mixer correctness.

## Decisions resolved in the module specifications

- Supported multitrack RTMP packet format and producer/FFmpeg compatibility,
  verified against the actual owning implementations.
- Incoming channel-layout handling and the explicit conversion into stereo
  mixer strips; incoming tracks and PCM channels are different identities.
- Initial bus membership for imported projects, track discovery/removal and
  solo scope; newly discovered tracks must not unexpectedly duplicate program
  audio in an existing mix.
- Live apply semantics, meter freshness, queue/backpressure handling, and
  timestamp-based alignment between audio tracks and video.

The completed implementation and historical verification results are recorded in
[tasks/todo.md](../tasks/todo.md).

## Implemented verification

- Parser tests cover legacy, batched Enhanced multitrack, malformed lengths and
  explicit unsupported packet/FourCC errors. ModEx is not supported.
- Real FFmpeg AAC decode preserves two tracks and distinct L/R tones.
- Exact PCM tests cover isolated buses, gain, mute, solo, balance, master controls,
  saturation, timestamp gaps, duplicate blocks and queue bounds.
- The real loopback scene-mixer test checks independent sources and tracks through
  two destinations, live routes/mute/scene edits, applied meters and reconnect.
- This is stereo mixing, not multichannel destination encoding or monitoring
  playback. Long-duration drift and production A/V latency remain unqualified.

Run the real decoder and complete scene-mixer acceptance tests on a host with
FFmpeg and a working GPU adapter:

```sh
cargo test -p acheflow-server real_multitrack_aac_preserves_each_track_and_stereo_channels --locked -- --ignored --nocapture
cargo test -p acheflow-server real_scene_mixer_multitrack_sources_and_live_destination_routes --locked -- --ignored --nocapture
```
