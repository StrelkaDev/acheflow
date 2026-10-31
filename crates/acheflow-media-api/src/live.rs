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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveStatus {
    pub workspace_id: Option<String>,
    pub rtmp_address: String,
    pub sources: Vec<SourceStatus>,
    pub formats: Vec<FormatStatus>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub id: u32,
    pub connected: bool,
    pub frames: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    pub audio: bool,
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputStatus {
    pub name: String,
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
