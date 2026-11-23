//! Persistent scene mixers and per-destination stereo bus routes.
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROGRAM_BUS: u32 = 1;
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_FRAMES: usize = 960;
pub const AUDIO_SAMPLES: usize = AUDIO_FRAMES * 2;
pub const MAX_SYNC_OFFSET_MS: i32 = 250;
pub const MAX_MIXER_EDITS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixerRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub edits: Vec<MixerEdit>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MixerAck {
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum MixerEdit {
    Strip {
        scene: u32,
        key: TrackKey,
        change: StripChange,
    },
    Bus {
        scene: u32,
        bus: u32,
        change: BusChange,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StripChange {
    Gain(u32),
    Pan(i32),
    Mute(bool),
    Solo(bool),
    SyncMode(SyncMode),
    SyncOffset(i32),
    ManualDelay(i32),
    Send { bus: u32, gain: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusChange {
    Gain(u32),
    Mute(bool),
    Name(String),
    Add,
    Remove,
}

impl MixerEdit {
    pub fn scene_id(&self) -> u32 {
        match self {
            Self::Strip { scene, .. } | Self::Bus { scene, .. } => *scene,
        }
    }

    pub fn apply(&self, workspace: &mut Workspace) -> Result<bool, String> {
        let scene_id = self.scene_id();
        let scene = workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.id == scene_id)
            .ok_or("Mixer scene no longer exists")?;
        match self {
            Self::Strip { key, change, .. } => {
                if !workspace
                    .sources
                    .iter()
                    .any(|source| source.id == key.source_id && source.kind.has_audio_input())
                {
                    return Err("Mixer input no longer exists".into());
                }
                if let StripChange::Send { bus, .. } = change
                    && !scene.audio.buses.iter().any(|entry| entry.id == *bus)
                {
                    return Err("Audio send bus no longer exists".into());
                }
                let existing = scene
                    .audio
                    .strips
                    .iter_mut()
                    .find(|strip| strip.key() == *key);
                let mut next = existing.as_deref().cloned().unwrap_or_else(|| {
                    AudioStrip::new(
                        *key,
                        key.track_id == 0
                            && scene
                                .layers
                                .iter()
                                .any(|layer| layer.source_id == key.source_id),
                    )
                });
                let before = next.clone();
                match *change {
                    StripChange::Gain(value) => next.gain = value,
                    StripChange::Pan(value) => next.pan = value,
                    StripChange::Mute(value) => next.muted = value,
                    StripChange::Solo(value) => next.solo = value,
                    StripChange::SyncMode(value) => next.sync_mode = value,
                    StripChange::SyncOffset(value) => next.sync_offset_ms = value,
                    StripChange::ManualDelay(value) => {
                        next.sync_mode = SyncMode::Manual;
                        next.sync_offset_ms = value;
                    }
                    StripChange::Send { bus, gain: 0 } => {
                        next.sends.remove(&bus);
                    }
                    StripChange::Send { bus, gain } => {
                        next.sends.insert(bus, gain);
                    }
                }
                if before == next {
                    return Ok(false);
                }
                if let Some(existing) = existing {
                    *existing = next;
                } else {
                    scene.audio.strips.push(next);
                }
            }
            Self::Bus { bus, change, .. } => {
                let existing = scene.audio.buses.iter_mut().find(|entry| entry.id == *bus);
                if *change == BusChange::Add {
                    if *bus == 0 || existing.is_some() || scene.audio.buses.len() >= 8 {
                        return Err("Cannot add this audio bus".into());
                    }
                    scene.audio.buses.push(AudioBus {
                        id: *bus,
                        name: format!("Bus {bus}"),
                        ..Default::default()
                    });
                } else {
                    let existing = existing.ok_or("Audio bus no longer exists")?;
                    match change {
                        BusChange::Gain(value) if existing.gain != *value => existing.gain = *value,
                        BusChange::Mute(value) if existing.muted != *value => {
                            existing.muted = *value
                        }
                        BusChange::Name(value) if existing.name != *value => {
                            existing.name.clone_from(value)
                        }
                        BusChange::Remove if *bus != PROGRAM_BUS => {
                            workspace.remove_audio_bus(scene_id, *bus)
                        }
                        _ => return Ok(false),
                    }
                }
            }
        }
        Ok(true)
    }

    /// Coalesce successive values from a single control without reordering
    /// edits across controls, bus creation or removal.
    pub fn replaces(&self, previous: &Self) -> bool {
        match (self, previous) {
            (
                Self::Strip { scene, key, change },
                Self::Strip {
                    scene: old_scene,
                    key: old_key,
                    change: old,
                },
            ) => {
                scene == old_scene
                    && key == old_key
                    && match (change, old) {
                        (StripChange::Send { bus, .. }, StripChange::Send { bus: old_bus, .. }) => {
                            bus == old_bus
                        }
                        _ => std::mem::discriminant(change) == std::mem::discriminant(old),
                    }
            }
            (
                Self::Bus { scene, bus, change },
                Self::Bus {
                    scene: old_scene,
                    bus: old_bus,
                    change: old,
                },
            ) => {
                scene == old_scene
                    && bus == old_bus
                    && !matches!(change, BusChange::Add | BusChange::Remove)
                    && std::mem::discriminant(change) == std::mem::discriminant(old)
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    #[default]
    Auto,
    AutoTrim,
    Manual,
}

impl SyncMode {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

fn zero_offset(value: &i32) -> bool {
    *value == 0
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackSyncStatus {
    pub key: TrackKey,
    pub mode: SyncMode,
    pub tracking_pts: bool,
    pub buffering: bool,
    pub video_delay_limited: bool,
    pub offset_ms: i32,
    pub audio_delay_ms: u32,
    pub video_delay_ms: u32,
    pub estimated_clock_offset_ms: Option<f64>,
}

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
    #[serde(skip_serializing_if = "SyncMode::is_auto")]
    pub sync_mode: SyncMode,
    #[serde(skip_serializing_if = "zero_offset")]
    pub sync_offset_ms: i32,
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
            sync_mode: SyncMode::Auto,
            sync_offset_ms: 0,
        }
    }
}
impl AudioStrip {
    pub fn active_sync_offset(&self) -> i32 {
        if self.sync_mode == SyncMode::Auto {
            0
        } else {
            self.sync_offset_ms
        }
    }
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
                || !(-MAX_SYNC_OFFSET_MS..=MAX_SYNC_OFFSET_MS).contains(&strip.sync_offset_ms)
                || strip
                    .sends
                    .iter()
                    .any(|(bus, gain)| !buses.contains(bus) || *gain > 200)
            {
                return Err("Invalid audio strip, source, gain, pan, send or sync offset".into());
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

    /// Advancing a track is represented by delaying its shared video. Other
    /// tracks from that source receive the same baseline video delay.
    pub fn video_delay_ms(&self, source_id: u32) -> u32 {
        self.strips
            .iter()
            .filter(|strip| strip.source_id == source_id)
            .map(|strip| strip.active_sync_offset().saturating_neg().max(0) as u32)
            .max()
            .unwrap_or(0)
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
    #[serde(default)]
    pub held_peak: [f32; 2],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recently_clipped: bool,
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

    #[test]
    fn video_delay_uses_the_largest_active_advance_on_the_source() {
        let mut scene = SceneAudio::default();
        for (track_id, mode, offset) in [
            (0, SyncMode::Auto, -200),
            (1, SyncMode::AutoTrim, -110),
            (2, SyncMode::Manual, 30),
        ] {
            scene.strips.push(AudioStrip {
                source_id: 1,
                track_id,
                sync_mode: mode,
                sync_offset_ms: offset,
                ..Default::default()
            });
        }
        assert_eq!(scene.video_delay_ms(1), 110);
        assert_eq!(scene.video_delay_ms(2), 0);
        scene.strips[1].sync_offset_ms = -MAX_SYNC_OFFSET_MS - 1;
        assert!(scene.validate(&[1]).is_err());
        assert_eq!(
            serde_json::from_str::<AudioStrip>("{}").unwrap().sync_mode,
            SyncMode::Auto
        );
    }
}
