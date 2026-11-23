use crate::{
    visual::SourceKind,
    workspace::{
        BitrateMode, Layer, Orientation, Output, OutputProtocol, Resolution, Scene, Source,
        Workspace,
    },
};
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputTarget>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputTarget {
    pub orientation: Orientation,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputAction {
    Start,
    Restart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManageOutputsRequest {
    pub workspace_id: String,
    pub action: OutputAction,
    #[serde(default)]
    pub output: Option<OutputTarget>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePreviewRequest {
    pub workspace_id: String,
    pub source_id: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeSceneRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub orientation: Orientation,
    pub scene_id: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TakeSceneAck {
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputProfile {
    #[serde(default)]
    pub protocol: OutputProtocol,
    pub name: String,
    pub url: String,
    pub key: String,
    pub resolution: Resolution,
    pub quality: String,
    pub codec: String,
    pub encoder: Option<String>,
    pub bitrate_kbps: u32,
    pub bitrate_mode: BitrateMode,
    pub keyframe_interval_secs: u32,
}

impl OutputProfile {
    pub fn of(output: &Output) -> Self {
        Self {
            protocol: output.protocol,
            name: output.name.clone(),
            url: output.url.clone(),
            key: output.key.clone(),
            resolution: output.resolution.clone(),
            quality: output.quality.clone(),
            codec: output.codec.clone(),
            encoder: output.encoder.clone(),
            bitrate_kbps: output.bitrate_kbps,
            bitrate_mode: output.bitrate_mode,
            keyframe_interval_secs: output.keyframe_interval_secs,
        }
    }

    pub fn apply(&self, output: &mut Output) {
        output.protocol = self.protocol;
        output.name.clone_from(&self.name);
        output.url.clone_from(&self.url);
        output.key.clone_from(&self.key);
        output.resolution.clone_from(&self.resolution);
        output.quality.clone_from(&self.quality);
        output.codec.clone_from(&self.codec);
        output.encoder.clone_from(&self.encoder);
        output.bitrate_kbps = self.bitrate_kbps;
        output.bitrate_mode = self.bitrate_mode;
        output.keyframe_interval_secs = self.keyframe_interval_secs;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputProfileRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub target: OutputTarget,
    pub profile: OutputProfile,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct OutputProfileAck {
    pub revision: u64,
}

impl OutputProfileRequest {
    pub fn apply(&self, workspace: &mut Workspace) -> Result<(), String> {
        let output = workspace
            .stream_mut(self.target.orientation)
            .outputs
            .get_mut(self.target.index)
            .ok_or("This output is not saved on the server")?;
        self.profile.apply(output);
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneContent {
    pub name: String,
    pub orientation: Orientation,
    pub layers: Vec<Layer>,
    pub spatial: Option<crate::spatial::SpatialScene>,
}

impl SceneContent {
    pub fn of(scene: &Scene) -> Self {
        Self {
            name: scene.name.clone(),
            orientation: scene.orientation,
            layers: scene.layers.clone(),
            spatial: scene.spatial.clone(),
        }
    }

    pub fn unchanged(draft: &Scene, saved: &Scene, base: &crate::workspace::Resolution) -> bool {
        let default;
        let saved_spatial = if saved.spatial.is_none() && draft.spatial.is_some() {
            default = crate::spatial::SpatialScene::from_canvas(base);
            Some(&default)
        } else {
            saved.spatial.as_ref()
        };
        draft.orientation == saved.orientation
            && draft.name == saved.name
            && draft.layers == saved.layers
            && draft.spatial.as_ref() == saved_spatial
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneEdit {
    Register(Scene),
    Content(SceneContent),
    Remove,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSettingsRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub scene_id: u32,
    pub edit: SceneEdit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SceneSettingsAck {
    pub revision: u64,
}

impl SceneEdit {
    pub fn apply(&self, workspace: &mut Workspace, scene_id: u32) -> Result<(), String> {
        if let Self::Register(scene) = self {
            if scene.id != scene_id || workspace.scenes.iter().any(|entry| entry.id == scene_id) {
                return Err("This scene ID is already in use or invalid".into());
            }
            workspace.scenes.push(scene.clone());
            return Ok(());
        }
        let scene = workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.id == scene_id)
            .ok_or("This scene is not saved on the server")?;
        match self {
            Self::Content(content) => {
                if content.orientation != scene.orientation {
                    return Err("The saved scene belongs to a different format".into());
                }
                scene.name.clone_from(&content.name);
                scene.layers.clone_from(&content.layers);
                scene.spatial.clone_from(&content.spatial);
            }
            Self::Remove => {
                let orientation = scene.orientation;
                let was_selected = workspace.active(orientation) == Some(scene_id);
                workspace.remove_scene(scene_id);
                if was_selected {
                    workspace.set_active(orientation, None);
                }
            }
            Self::Register(_) => unreachable!(),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCapture {
    pub kind: SourceKind,
    pub url: String,
    pub key: String,
}

impl SourceCapture {
    pub fn of(source: &Source) -> Self {
        Self {
            kind: source.kind.clone(),
            url: source.url.clone(),
            key: source.key.clone(),
        }
    }

    pub fn matches(&self, source: &Source) -> bool {
        self.kind == source.kind && self.url == source.url && self.key == source.key
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEdit {
    Name(String),
    PreviewWidth(u32),
    PreviewHeight(u32),
    VisualSettings(SourceKind),
    Capture(SourceCapture),
    Register(Source),
    Remove,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSettingsRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub source_id: u32,
    pub edit: SourceEdit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SourceSettingsAck {
    pub revision: u64,
}

impl SourceEdit {
    pub fn renews_capture(&self) -> bool {
        matches!(self, Self::Capture(_) | Self::Register(_) | Self::Remove)
    }

    pub fn apply(&self, workspace: &mut Workspace, source_id: u32) -> Result<bool, String> {
        if let Self::Register(source) = self {
            if source.id != source_id || workspace.sources.iter().any(|entry| entry.id == source_id)
            {
                return Err("This source ID is already in use or invalid".into());
            }
            workspace.sources.push(source.clone());
            return Ok(true);
        }
        let source = workspace
            .sources
            .iter_mut()
            .find(|source| source.id == source_id)
            .ok_or("This source is not saved on the server")?;
        match self {
            Self::Remove => workspace.remove_source(source_id),
            Self::Name(name) if name.trim().is_empty() => {
                return Err("Source name is required".into());
            }
            Self::Name(name) if source.name != *name => source.name.clone_from(name),
            Self::PreviewWidth(width) if source.resolution.width != *width => {
                source.resolution.width = *width
            }
            Self::PreviewHeight(height) if source.resolution.height != *height => {
                source.resolution.height = *height
            }
            Self::VisualSettings(kind) => {
                if source.kind.is_media_input()
                    || std::mem::discriminant(&source.kind) != std::mem::discriminant(kind)
                {
                    return Err("Only settings for the existing overlay type can be saved".into());
                }
                if source.kind == *kind {
                    return Ok(false);
                }
                source.kind.clone_from(kind);
            }
            Self::Capture(capture) => {
                if !source.kind.is_media_input()
                    || std::mem::discriminant(&source.kind) != std::mem::discriminant(&capture.kind)
                {
                    return Err(
                        "A source's capture type cannot be changed by reconnecting it".into(),
                    );
                }
                if capture.matches(source) {
                    return Ok(false);
                }
                source.kind.clone_from(&capture.kind);
                source.url.clone_from(&capture.url);
                source.key.clone_from(&capture.key);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl TakeSceneRequest {
    pub fn apply(&self, workspace: &mut Workspace) -> Result<(), String> {
        if !workspace
            .scenes
            .iter()
            .any(|scene| scene.id == self.scene_id && scene.orientation == self.orientation)
        {
            return Err("This scene is not saved for the selected format".into());
        }
        workspace.set_active(self.orientation, Some(self.scene_id));
        Ok(())
    }
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gpus: Vec<GpuLoadSample>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuLoadSample {
    /// A server-local key for combining samples from the same device.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder_percent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unified_memory: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_scene: Option<AppliedScene>,
    #[serde(default)]
    pub time_seconds: f64,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub orientation: Orientation,
    pub frames: u64,
    pub outputs: Vec<OutputStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preview_profiles: Vec<OutputPreviewProfile>,
    #[serde(default)]
    pub audio_buses: Vec<crate::audio::AudioBusStatus>,
    #[serde(default)]
    pub audio_sync: Vec<crate::audio::TrackSyncStatus>,
    #[serde(default)]
    pub render_load: RenderLoad,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedScene {
    pub id: u32,
    pub name: String,
}

pub const ENCODED_PREVIEW_SNAPSHOT_FPS: u32 = 15;

/// Public framing and encoder settings for a configured output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputPreviewProfile {
    #[serde(default)]
    pub protocol: OutputProtocol,
    pub index: usize,
    pub name: String,
    pub resolution: crate::workspace::Resolution,
    pub codec: String,
    pub bitrate_kbps: u32,
    pub bitrate_mode: crate::workspace::BitrateMode,
    pub fps: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutputTrackTiming {
    pub video: Option<TrackEncodeTiming>,
    pub audio: Option<TrackEncodeTiming>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackEncodeTiming {
    pub pipeline_p95_ms: f32,
    /// FFmpeg reports individual stages only when they are significant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encode_p95_ms: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputStatus {
    #[serde(default)]
    pub protocol: OutputProtocol,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restart_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect_attempt: Option<u8>,
    pub frames: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_fps: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_timing: Option<OutputTrackTiming>,
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
