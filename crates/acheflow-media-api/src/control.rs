use crate::{spatial::SpatialScene, workspace::Scene};
use acheflow_extension_api::{
    audience::{AudiencePoint, AudienceSnapshot},
    interaction::{ChatPage, CommandRule, Poll, PollRequest},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExtensionStatus {
    pub id: String,
    pub name: String,
    pub state: String,
    pub error: Option<String>,
    pub capabilities: Vec<String>,
    pub account: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthorizationStatus {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlSnapshot {
    pub panels: Vec<acheflow_extension_api::ExtensionPanel>,
    pub edit_revision: u64,
    pub workspace_id: Option<String>,
    pub revision: u64,
    pub chat: ChatPage,
    pub audience: AudienceSnapshot,
    pub history: Vec<AudiencePoint>,
    pub polls: Vec<Poll>,
    pub commands: Vec<CommandRule>,
    pub extensions: Vec<ExtensionStatus>,
    pub authorization: Option<AuthorizationStatus>,
    pub scenes: Vec<Scene>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlAction {
    ExtensionEvent {
        id: String,
        action_id: String,
        value: String,
    },
    SendChat {
        channel_id: String,
        text: String,
    },
    CreatePoll {
        poll: PollRequest,
    },
    ClosePoll {
        id: u64,
        cancel: bool,
    },
    SetCommand {
        command: CommandRule,
    },
    SetScene {
        scene_id: u32,
        spatial: SpatialScene,
    },
    SetExtension {
        id: String,
        enabled: bool,
    },
    AuthorizeTwitch {
        client_id: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub workspace_id: String,
    pub request_id: String,
    pub expected_revision: u64,
    pub action: ControlAction,
}
