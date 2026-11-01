//! Persistent scene mixers and per-destination stereo bus routes.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROGRAM_BUS: u32 = 1;
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_FRAMES: usize = 960;
pub const AUDIO_SAMPLES: usize = AUDIO_FRAMES * 2;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct TrackKey {
    pub source_id: u32,
    pub track_id: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioStrip {
    pub source_id: u32,
    pub track_id: u8,
    pub gain: u32,
    pub muted: bool,
    pub solo: bool,
    pub pan: i32,
    pub sends: BTreeMap<u32, u32>,
}
impl Default for AudioStrip {
    fn default() -> Self {
        Self {
            source_id: 0,
            track_id: 0,
            gain: 100,
            muted: false,
            solo: false,
            pan: 0,
            sends: BTreeMap::new(),
        }
    }
}
impl AudioStrip {
    pub fn key(&self) -> TrackKey {
        TrackKey {
            source_id: self.source_id,
            track_id: self.track_id,
        }
    }
    pub fn new(key: TrackKey, program: bool) -> Self {
        Self {
            source_id: key.source_id,
            track_id: key.track_id,
            sends: if program {
                BTreeMap::from([(PROGRAM_BUS, 100)])
            } else {
                BTreeMap::new()
            },
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioBus {
    pub id: u32,
    pub name: String,
    pub gain: u32,
    pub muted: bool,
}
impl Default for AudioBus {
    fn default() -> Self {
        Self {
            id: PROGRAM_BUS,
            name: "Program".into(),
            gain: 100,
            muted: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SceneAudio {
    pub strips: Vec<AudioStrip>,
    pub buses: Vec<AudioBus>,
}
impl Default for SceneAudio {
    fn default() -> Self {
        Self {
            strips: vec![],
            buses: vec![AudioBus::default()],
        }
    }
}
impl SceneAudio {
    pub fn validate(&self, sources: &[u32]) -> Result<(), String> {
        use std::collections::HashSet;
        let mut buses = HashSet::new();
        if self.buses.is_empty() || self.buses.len() > 8 || self.strips.len() > 64 {
            return Err("A scene supports 1–8 audio buses and at most 64 strips".into());
        }
        for bus in &self.buses {
            if bus.id == 0
                || !buses.insert(bus.id)
                || bus.name.trim().is_empty()
                || bus.name.len() > 128
                || bus.gain > 200
            {
                return Err("Invalid or duplicate audio bus".into());
            }
        }
        if !buses.contains(&PROGRAM_BUS) {
            return Err("The Program audio bus (ID 1) is required".into());
        }
        let mut tracks = HashSet::new();
        for strip in &self.strips {
            if !sources.contains(&strip.source_id)
                || !tracks.insert(strip.key())
                || strip.gain > 200
                || !(-100..=100).contains(&strip.pan)
                || strip
                    .sends
                    .iter()
                    .any(|(bus, gain)| !buses.contains(bus) || *gain > 200)
            {
                return Err("Invalid audio strip, source, gain, pan or send".into());
            }
        }
        Ok(())
    }
    pub fn effective_strips(
        &self,
        scene_sources: impl IntoIterator<Item = u32>,
    ) -> BTreeMap<TrackKey, AudioStrip> {
        let mut strips = scene_sources
            .into_iter()
            .map(|source_id| {
                let key = TrackKey {
                    source_id,
                    track_id: 0,
                };
                (key, AudioStrip::new(key, true))
            })
            .collect::<BTreeMap<_, _>>();
        for strip in &self.strips {
            strips.insert(strip.key(), strip.clone());
        }
        strips
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputAudio {
    pub scene_buses: BTreeMap<u32, u32>,
    pub gain: u32,
    pub muted: bool,
}
impl Default for OutputAudio {
    fn default() -> Self {
        Self {
            scene_buses: BTreeMap::new(),
            gain: 100,
            muted: false,
        }
    }
}
impl OutputAudio {
    pub fn bus(&self, scene: u32) -> u32 {
        self.scene_buses.get(&scene).copied().unwrap_or(PROGRAM_BUS)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioMeter {
    pub peak: [f32; 2],
    pub rms: [f32; 2],
    pub clipped_samples: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioTrackStatus {
    pub track_id: u8,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub active: bool,
    pub meter: AudioMeter,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBusStatus {
    pub scene_id: u32,
    pub bus_id: u32,
    pub name: String,
    pub meter: AudioMeter,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_scene_defaults_and_explicit_sends_do_not_duplicate_audio() {
        let mut scene = SceneAudio::default();
        assert_eq!(scene.effective_strips([4, 4]).len(), 1);
        scene.strips.push(AudioStrip::new(
            TrackKey {
                source_id: 4,
                track_id: 0,
            },
            false,
        ));
        assert!(
            scene.effective_strips([4])[&TrackKey {
                source_id: 4,
                track_id: 0
            }]
                .sends
                .is_empty()
        );
        assert!(scene.validate(&[4]).is_ok());
        scene.strips[0].sends.insert(999, 100);
        assert!(scene.validate(&[4]).is_err());
        scene.strips[0].sends.clear();
        scene.strips[0].pan = 101;
        assert!(scene.validate(&[4]).is_err());
    }
}
