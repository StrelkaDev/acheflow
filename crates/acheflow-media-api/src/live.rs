use crate::workspace::{Orientation, Workspace};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub modes: Vec<crate::visual::CameraMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraInventory {
    pub supported: bool,
    pub devices: Vec<CameraDevice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub modes: Vec<crate::visual::MicrophoneMode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrophoneInventory {
    pub supported: bool,
    pub devices: Vec<MicrophoneDevice>,
}

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
    pub activated: bool,
    #[serde(default)]
    pub edit_revision: u64,
    pub workspace_id: Option<String>,
    pub rtmp_address: String,
    #[serde(default)]
    pub rtmp_tls: bool,
    pub sources: Vec<SourceStatus>,
    pub formats: Vec<FormatStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_load: Vec<ServerLoadSample>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerLoadSample {
    pub time_ms: u64,
    pub cpu_percent: f32,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busiest_gpu_percent: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderLoad {
    pub target_fps: u32,
    pub observed_fps: f64,
    pub p95_ms: f64,
    pub deadline_miss_percent: f64,
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
    #[serde(default)]
    pub audio_sync: Vec<crate::audio::TrackSyncStatus>,
    #[serde(default)]
    pub render_load: RenderLoad,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_attempt: Option<u8>,
    pub frames: u64,
    #[serde(default)]
    pub repeated_video_frames: u64,
    #[serde(default)]
    pub inserted_audio_blocks: u64,
    #[serde(default)]
    pub reconnects: u64,
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
