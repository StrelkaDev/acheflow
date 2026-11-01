//! Fixed-gain stereo scene buses and bounded timestamp-based audio playout.
use acheflow_media_api::audio::{AUDIO_SAMPLES, AudioMeter, OutputAudio, SceneAudio, TrackKey};
use std::collections::BTreeMap;

pub struct MixedBus {
    pub samples: Vec<f32>,
    pub meter: AudioMeter,
}

pub fn render_buses(
    mixer: &SceneAudio,
    scene_sources: impl IntoIterator<Item = u32>,
    pcm: &BTreeMap<TrackKey, Vec<i16>>,
) -> BTreeMap<u32, MixedBus> {
    let strips = mixer.effective_strips(scene_sources);
    let solo = strips.values().any(|s| s.solo);
    let mut buses = mixer
        .buses
        .iter()
        .map(|bus| (bus.id, vec![0f32; AUDIO_SAMPLES]))
        .collect::<BTreeMap<_, _>>();
    for strip in strips.values().filter(|s| !s.muted && (!solo || s.solo)) {
        let Some(samples) = pcm.get(&strip.key()) else {
            continue;
        };
        let gain = strip.gain as f32 / 100.;
        let balance = [
            1. - strip.pan.max(0) as f32 / 100.,
            1. + strip.pan.min(0) as f32 / 100.,
        ];
        for (bus, send) in &strip.sends {
            let Some(out) = buses.get_mut(bus) else {
                continue;
            };
            for (i, (out, sample)) in out.iter_mut().zip(samples).enumerate() {
                *out += f32::from(*sample) / 32768. * gain * (*send as f32 / 100.) * balance[i % 2];
            }
        }
    }
    mixer
        .buses
        .iter()
        .map(|bus| {
            let mut samples = buses.remove(&bus.id).unwrap();
            let gain = if bus.muted {
                0.
            } else {
                bus.gain as f32 / 100.
            };
            for sample in &mut samples {
                *sample *= gain;
            }
            let meter = float_meter(&samples);
            (bus.id, MixedBus { samples, meter })
        })
        .collect()
}

pub fn destination_pcm(bus: &MixedBus, route: &OutputAudio) -> (Vec<u8>, AudioMeter) {
    let gain = if route.muted {
        0.
    } else {
        route.gain as f32 / 100.
    };
    let samples = bus.samples.iter().map(|s| s * gain).collect::<Vec<_>>();
    let meter = float_meter(&samples);
    let bytes = samples
        .into_iter()
        .flat_map(|s| {
            ((s * 32768.).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes()
        })
        .collect();
    (bytes, meter)
}

fn float_meter(samples: &[f32]) -> AudioMeter {
    let mut meter = AudioMeter::default();
    let mut power = [0f64; 2];
    for (i, &sample) in samples.iter().enumerate() {
        meter.peak[i % 2] = meter.peak[i % 2].max(sample.abs());
        power[i % 2] += f64::from(sample).powi(2);
        if !(-32768. ..=32767.).contains(&(sample * 32768.).round()) {
            meter.clipped_samples += 1;
        }
    }
    for (channel, power) in power.into_iter().enumerate() {
        meter.rms[channel] = (power / (samples.len() / 2).max(1) as f64).sqrt() as f32;
    }
    meter
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::audio::{AudioBus, AudioStrip};
    fn constant(left: i16, right: i16) -> Vec<i16> {
        [left, right].repeat(AUDIO_SAMPLES / 2)
    }
    #[test]
    fn playout_preserves_timestamps_gaps_and_bounds_latency() {
        let mut queue = Playout::default();
        let frame = |pts, value| {
            Arc::new(crate::audio_input::AudioFrame {
                pts,
                samples: Arc::new(vec![value; AUDIO_SAMPLES]),
            })
        };
        queue.push(frame(960, 12));
        assert!(queue.render(0).iter().all(|s| *s == 0));
        let pcm = queue.render(480);
        assert!(pcm[..960].iter().all(|s| *s == 0));
        assert!(pcm[960..].iter().all(|s| *s == 12));
        queue.push(frame(960, 99));
        assert_eq!(queue.frames.len(), 1);
        queue.push(frame(1920, 34));
        let pcm = queue.render(1440);
        assert_eq!(pcm[0], 12);
        assert_eq!(pcm[960], 34);
        assert!(queue.render(10000).iter().all(|s| *s == 0));
        assert!(queue.frames.is_empty());
        for n in 0..100 {
            queue.push(frame(n * 960, 1));
        }
        assert_eq!(queue.frames.len(), 16);
        assert_eq!(queue.frames.front().unwrap().pts, 84 * 960);
    }
    #[test]
    fn buses_keep_stereo_and_only_receive_their_selected_strips() {
        let a = TrackKey {
            source_id: 1,
            track_id: 0,
        };
        let b = TrackKey {
            source_id: 1,
            track_id: 1,
        };
        let mut mixer = SceneAudio::default();
        mixer.buses.push(AudioBus {
            id: 2,
            name: "Clean".into(),
            ..AudioBus::default()
        });
        mixer.strips = vec![
            AudioStrip::new(a, true),
            AudioStrip {
                sends: BTreeMap::from([(2, 100)]),
                ..AudioStrip::new(b, false)
            },
        ];
        let pcm = BTreeMap::from([(a, constant(1000, -2000)), (b, constant(3000, 4000))]);
        let buses = render_buses(&mixer, [1, 1], &pcm);
        assert_eq!(
            &destination_pcm(&buses[&1], &OutputAudio::default()).0[..4],
            &[0xe8, 0x03, 0x30, 0xf8]
        );
        assert_eq!(
            &destination_pcm(&buses[&2], &OutputAudio::default()).0[..4],
            &[0xb8, 0x0b, 0xa0, 0x0f]
        );
        assert_eq!(buses[&1].meter.peak[0], 1000. / 32768.);
    }
    #[test]
    fn controls_have_fixed_gain_mute_solo_pan_and_final_output_clipping() {
        let a = TrackKey {
            source_id: 1,
            track_id: 0,
        };
        let b = TrackKey {
            source_id: 2,
            track_id: 0,
        };
        let pcm = BTreeMap::from([(a, constant(10000, 20000)), (b, constant(10000, 10000))]);
        let mut mixer = SceneAudio {
            strips: vec![AudioStrip {
                gain: 50,
                pan: -100,
                solo: true,
                ..AudioStrip::new(a, true)
            }],
            ..Default::default()
        };
        let bus = render_buses(&mixer, [1, 2], &pcm).remove(&1).unwrap();
        assert_eq!(bus.samples[0], 5000. / 32768.);
        assert_eq!(bus.samples[1], 0.);
        let route = OutputAudio {
            gain: 200,
            ..OutputAudio::default()
        };
        assert_eq!(&destination_pcm(&bus, &route).0[..4], &[0x10, 0x27, 0, 0]);
        mixer.strips[0].muted = true;
        assert!(
            render_buses(&mixer, [1, 2], &pcm)[&1]
                .samples
                .iter()
                .all(|s| *s == 0.)
        );
        mixer.strips.clear();
        mixer.buses[0].gain = 200;
        let bus = render_buses(&mixer, [1, 2], &pcm).remove(&1).unwrap();
        let (bytes, meter) = destination_pcm(&bus, &OutputAudio::default());
        assert_eq!(&bytes[..4], &[0xff, 0x7f, 0xff, 0x7f]);
        assert_eq!(meter.clipped_samples, AUDIO_SAMPLES as u64);
        assert!(
            destination_pcm(
                &bus,
                &OutputAudio {
                    muted: true,
                    ..OutputAudio::default()
                }
            )
            .0
            .iter()
            .all(|b| *b == 0)
        );
        mixer.buses[0].muted = true;
        assert!(
            render_buses(&mixer, [1, 2], &pcm)[&1]
                .samples
                .iter()
                .all(|s| *s == 0.)
        );
    }
}

/// Bounded PCM timeline. Missing samples become silence; old data never adds latency.
#[derive(Default)]
struct Playout {
    frames: std::collections::VecDeque<std::sync::Arc<crate::audio_input::AudioFrame>>,
}
impl Playout {
    fn push(&mut self, frame: std::sync::Arc<crate::audio_input::AudioFrame>) {
        if self.frames.back().is_some_and(|last| frame.pts <= last.pts) {
            return;
        }
        self.frames.push_back(frame);
        while self.frames.len() > 16 {
            self.frames.pop_front();
        }
    }
    fn render(&mut self, pts: i64) -> Vec<i16> {
        use acheflow_media_api::audio::AUDIO_FRAMES;
        let end = pts + AUDIO_FRAMES as i64;
        while self
            .frames
            .front()
            .is_some_and(|f| f.pts + (f.samples.len() / 2) as i64 <= pts)
        {
            self.frames.pop_front();
        }
        let mut out = vec![0; AUDIO_SAMPLES];
        for frame in &self.frames {
            let start = pts.max(frame.pts);
            let stop = end.min(frame.pts + (frame.samples.len() / 2) as i64);
            if start < stop {
                let dst = (start - pts) as usize * 2;
                let src = (start - frame.pts) as usize * 2;
                let len = (stop - start) as usize * 2;
                out[dst..dst + len].copy_from_slice(&frame.samples[src..src + len]);
            }
        }
        out
    }
}

use acheflow_media_api::{audio::AudioBusStatus, live::OutputStatus, workspace::Orientation};
use std::sync::{Arc, Mutex, atomic::Ordering};
use tokio::sync::broadcast;
pub struct Destination {
    pub index: usize,
    pub sender: broadcast::Sender<Arc<Vec<u8>>>,
    pub status: Arc<Mutex<OutputStatus>>,
}
struct Subscription {
    track: Arc<crate::audio_input::AudioTrack>,
    receiver: broadcast::Receiver<Arc<crate::audio_input::AudioFrame>>,
    playout: Playout,
}

pub async fn run(
    runtime: crate::live::RuntimeState,
    orientation: Orientation,
    destinations: Vec<Destination>,
    status: Arc<Mutex<Vec<AudioBusStatus>>>,
    cancel: tokio_util::sync::CancellationToken,
) {
    use acheflow_media_api::audio::AUDIO_RATE;
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sources = BTreeMap::<u32, Arc<crate::ingest::Input>>::new();
    let mut clocks = BTreeMap::<u32, i64>::new();
    let mut subscriptions = BTreeMap::<TrackKey, Subscription>::new();
    loop {
        tokio::select! {_=cancel.cancelled()=>break,_=ticker.tick()=>{}}
        let now = (started.elapsed().as_millis() / 20) as i64 * 960;
        let inputs = runtime.inputs.lock().unwrap().clone();
        sources.retain(|id, old| {
            let keep = inputs.get(id).is_some_and(|input| {
                Arc::ptr_eq(input, old) && input.connected.load(Ordering::Acquire)
            });
            if !keep {
                clocks.remove(id);
                subscriptions.retain(|key, _| key.source_id != *id);
            }
            keep
        });
        let mut tracks = BTreeMap::new();
        for (&id, input) in &inputs {
            if !input.connected.load(Ordering::Acquire) {
                continue;
            }
            sources.insert(id, input.clone());
            for (&track_id, track) in input.audio_tracks.lock().unwrap().iter() {
                tracks.insert(
                    TrackKey {
                        source_id: id,
                        track_id,
                    },
                    track.clone(),
                );
            }
        }
        subscriptions.retain(|key, sub| {
            tracks
                .get(key)
                .is_some_and(|track| Arc::ptr_eq(track, &sub.track))
        });
        for (key, track) in tracks {
            subscriptions.entry(key).or_insert_with(|| Subscription {
                receiver: track.frames.subscribe(),
                track,
                playout: Playout::default(),
            });
        }
        let mut pcm = BTreeMap::new();
        for (key, sub) in &mut subscriptions {
            // A continuously publishing input cannot monopolize this mixer tick.
            for _ in 0..32 {
                match sub.receiver.try_recv() {
                    Ok(frame) => {
                        // Share a source clock across tracks, preserving their relative PTS.
                        // Rebase after a large discontinuity rather than accumulating delay.
                        let offset = clocks
                            .entry(key.source_id)
                            .or_insert(now + 2880 - frame.pts);
                        if (frame.pts + *offset - now).abs() > AUDIO_RATE as i64 / 2 {
                            *offset = now + 2880 - frame.pts;
                            sub.playout.frames.clear();
                        }
                        sub.playout.push(frame);
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => {
                        sub.playout.frames.clear();
                    }
                    Err(_) => break,
                }
            }
            if let Some(offset) = clocks.get(&key.source_id) {
                pcm.insert(*key, sub.playout.render(now - *offset));
            }
        }
        let workspace = runtime.workspace.read().unwrap().clone();
        let scene = workspace.as_ref().and_then(|w| {
            w.scenes
                .iter()
                .find(|s| Some(s.id) == w.active(orientation))
        });
        let buses = scene
            .map(|s| render_buses(&s.audio, s.layers.iter().map(|l| l.source_id), &pcm))
            .unwrap_or_default();
        *status.lock().unwrap() = scene
            .map(|s| {
                s.audio
                    .buses
                    .iter()
                    .map(|b| AudioBusStatus {
                        scene_id: s.id,
                        bus_id: b.id,
                        name: b.name.clone(),
                        meter: buses[&b.id].meter.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for dest in &destinations {
            let route = workspace
                .as_ref()
                .and_then(|w| w.stream(orientation).outputs.get(dest.index))
                .map(|o| &o.audio);
            let bus_id = scene.zip(route).map(|(s, r)| r.bus(s.id));
            let (bytes, meter) = bus_id
                .and_then(|id| buses.get(&id))
                .zip(route)
                .map(|(bus, r)| destination_pcm(bus, r))
                .unwrap_or_else(|| (vec![0; AUDIO_SAMPLES * 2], AudioMeter::default()));
            {
                let mut out = dest.status.lock().unwrap();
                out.audio_scene = scene.map(|s| s.id);
                out.audio_bus = bus_id;
                out.audio_meter = meter;
            }
            let _ = dest.sender.send(Arc::new(bytes));
        }
    }
}
