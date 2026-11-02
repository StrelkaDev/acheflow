use crate::workspace::{Orientation, Workspace};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub workspace: Workspace,
    pub formats: Vec<Orientation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestartOutputRequest {
    pub workspace_id: String,
    pub orientation: Orientation,
    pub index: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveStatus {
    #[serde(default)]
    pub edit_revision: u64,
    pub workspace_id: Option<String>,
    pub rtmp_address: String,
    #[serde(default)]
    pub rtmp_tls: bool,
    pub sources: Vec<SourceStatus>,
    pub formats: Vec<FormatStatus>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub id: u32,
    pub connected: bool,
    #[serde(default)]
    pub received_bytes: u64,
    #[serde(default)]
    pub bitrate_kbps: f64,
    pub frames: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub audio: bool,
    #[serde(default)]
    pub audio_tracks: Vec<crate::audio::AudioTrackStatus>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormatStatus {
    #[serde(default)]
    pub time_seconds: f64,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub orientation: Orientation,
    pub frames: u64,
    pub outputs: Vec<OutputStatus>,
    #[serde(default)]
    pub audio_buses: Vec<crate::audio::AudioBusStatus>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputStatus {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub sent_bytes: u64,
    #[serde(default)]
    pub bitrate_kbps: f64,
    pub name: String,
    #[serde(default)]
    pub encoder: Option<String>,
    #[serde(default)]
    pub hardware: bool,
    #[serde(default)]
    pub audio_scene: Option<u32>,
    #[serde(default)]
    pub audio_bus: Option<u32>,
    #[serde(default)]
    pub audio_meter: crate::audio::AudioMeter,
    pub state: OutputState,
    pub frames: u64,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputState {
    Starting,
    Streaming,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
