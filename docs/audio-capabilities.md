# Audio capability map

## Confirmed scope

- Audio can arrive from multiple RTMP sources and as multiple independent
  audio tracks inside a single compatible RTMP input.
- Each output selects one stereo mixer bus. Multiple independent audio tracks
  inside one output are outside this request.
- Mixer configurations belong to scenes. Destinations store a bus selection for
  each scene, so shared program/clean feeds do not require duplicate mixers.
- The current transport adapter, FFmpeg demuxer and compatible producer must
  preserve and identify incoming tracks. Unsupported multitrack input must be
  reported explicitly, not reduced silently to the first track.

## Initial audit (before implementation)

- `ingest.rs` maps only `0:a:0`, resampling it to 48 kHz stereo signed PCM.
- `compositor.rs` takes the sources used by a selected scene, averages their
  stereo PCM and sends the same mix to every output of that format.
- `output.rs` maps one audio input to AAC stereo at 128 kbps.
- The existing RTMP integration test publishes two sine tones, but its audio
  assertion checks only non-silence. It does not prove that both sources survive,
  that left/right remain distinct, or that output routing isolates channels.
- There is currently no persisted mixer, independent input-track model or
  per-output bus selection.

## Capability boundaries

| Module id | Responsibility | Depends on |
| --- | --- | --- |
| audio-ingest | Discover stable source/track identities, retain track metadata, decode each incoming track independently, expose arrival and level information, report unsupported input formats. | — |
| audio-mixer | Per-scene input strips keyed by source and track; gain, mute, solo and pan; stereo buses and sends; bus gain/mute and meters; bounded PCM processing and explicit clipping. | audio-ingest |
| output-audio-routing | Persist each destination's bus selection per scene, feed that bus to its AAC encoder, show the applied route, allow live mixer/routing changes without restarting video encoding. | audio-mixer |

Build order: audio-ingest → audio-mixer → output-audio-routing.

Shared workspace contracts must provide stable references and reject duplicate
or missing track/bus references. Legacy workspace imports require defaults so
existing sources, scenes and output configurations remain usable.

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

The user approved this scope and continuing without intermediate approval gates.
Implementation and verification are tracked in `tasks/todo.md`.

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
