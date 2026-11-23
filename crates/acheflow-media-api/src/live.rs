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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartSavedRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub scenes: [Option<u32>; 2],
    pub preview_only: bool,
    pub output: Option<OutputTarget>,
}

impl StartSavedRequest {
    pub fn programs(&self) -> impl Iterator<Item = (Orientation, u32)> + '_ {
        Orientation::ALL
            .into_iter()
            .zip(self.scenes)
            .filter_map(|(orientation, scene)| scene.map(|scene| (orientation, scene)))
    }

    pub fn apply(&self, workspace: &mut Workspace) -> Result<(), String> {
        for (orientation, scene_id) in self.programs() {
            set_program_scene(workspace, orientation, scene_id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopFormatRequest {
    pub workspace_id: String,
    pub orientation: Orientation,
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
pub struct FormatSettingsRequest {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub orientation: Orientation,
    pub edit: FormatSettingsEdit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FormatSettingsEdit {
    Canvas {
        base: Resolution,
        fps: u32,
    },
    AddOutput {
        index: usize,
        profile: OutputProfile,
        enabled: bool,
    },
    RemoveOutput {
        index: usize,
    },
    SetOutputEnabled {
        index: usize,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FormatSettingsAck {
    pub revision: u64,
}

impl FormatSettingsRequest {
    pub fn apply(&self, workspace: &mut Workspace) -> Result<(), String> {
        let outputs = &mut workspace.stream_mut(self.orientation).outputs;
        match &self.edit {
            FormatSettingsEdit::Canvas { base, fps } => {
                if !crate::workspace::valid_stream_size(base) || !(1..=120).contains(fps) {
                    return Err("Invalid base resolution or FPS".into());
                }
                if workspace.stream(self.orientation).base != *base {
                    workspace.resize_base(self.orientation, base.clone());
                }
                workspace.stream_mut(self.orientation).fps = *fps;
            }
            FormatSettingsEdit::AddOutput {
                index,
                profile,
                enabled,
            } => {
                if *index != outputs.len() {
                    return Err("conflict: Destination order changed".into());
                }
                let OutputProfile {
                    protocol,
                    name,
                    url,
                    key,
                    resolution,
                    quality,
                    codec,
                    encoder,
                    bitrate_kbps,
                    bitrate_mode,
                    keyframe_interval_secs,
                } = profile.clone();
                outputs.push(Output {
                    protocol,
                    name,
                    url,
                    key,
                    resolution,
                    quality,
                    codec,
                    encoder,
                    bitrate_kbps,
                    bitrate_mode,
                    keyframe_interval_secs,
                    enabled: *enabled,
                    audio: Default::default(),
                });
            }
            FormatSettingsEdit::RemoveOutput { index } => {
                if *index >= outputs.len() {
                    return Err("This output is not saved on the server".into());
                }
                outputs.remove(*index);
            }
            FormatSettingsEdit::SetOutputEnabled { index, enabled } => {
                outputs
                    .get_mut(*index)
                    .ok_or("This output is not saved on the server")?
                    .enabled = *enabled;
            }
        }
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
        set_program_scene(workspace, self.orientation, self.scene_id)
    }
}

fn set_program_scene(
    workspace: &mut Workspace,
    orientation: Orientation,
    scene_id: u32,
) -> Result<(), String> {
    if !workspace
        .scenes
        .iter()
        .any(|scene| scene.id == scene_id && scene.orientation == orientation)
    {
        return Err("This scene is not saved for the selected format".into());
    }
    workspace.set_active(orientation, Some(scene_id));
    Ok(())
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
pub const SERVER_LOAD_HISTORY: usize = 240;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaTelemetry {
    pub workspace_id: Option<String>,
    pub edit_revision: u64,
    pub meters: Vec<MeterReading>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render: Vec<RenderTelemetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_load: Vec<ServerLoadSample>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderTelemetry {
    pub orientation: Orientation,
    pub time_seconds: f64,
    pub load: RenderLoad,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeterReading {
    pub target: MeterTarget,
    pub meter: crate::audio::AudioMeter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeterTarget {
    Source {
        source_id: u32,
        track_id: u8,
    },
    Bus {
        orientation: Orientation,
        scene_id: u32,
        bus_id: u32,
    },
    Output {
        orientation: Orientation,
        index: usize,
    },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_profile: Option<GpuFrameProfile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuFrameProfile {
    pub scene_id: u32,
    pub passes: Vec<GpuPassTiming>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuPassTiming {
    pub stage: String,
    pub viewport_id: Option<u64>,
    pub layer_id: Option<u64>,
    pub source_id: Option<u32>,
    pub elapsed_ms: f64,
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
