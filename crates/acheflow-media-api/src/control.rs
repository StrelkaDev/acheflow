use crate::{spatial::SpatialScene, workspace::Scene};
use acheflow_extension_api::{
    audience::{AudiencePoint, AudienceSnapshot},
    interaction::{ChatPage, CommandRule, Poll, PollRequest},
};
use serde::{Deserialize, Serialize};

/// Trusted Studio operator HTTP API; external executors use authenticated MCP.
/// Provider credentials and raw platform payloads are never accepted here.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudioExecutorRequest {
    pub workspace_id: String,
    pub client_id: String,
    pub operation: acheflow_extension_api::client_execution::ExecutorRequest,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExtensionStatus {
    pub id: String,
    pub name: String,
    pub state: String,
    pub error: Option<String>,
    pub capabilities: Vec<String>,
    pub account: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub settings: Vec<acheflow_extension_api::integration::Setting>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ControlSnapshot {
    #[serde(default)]
    pub client_capabilities: Vec<acheflow_extension_api::client_execution::CapabilityGrant>,
    #[serde(default)]
    pub client_executors: Vec<acheflow_extension_api::client_execution::ExecutorStatus>,
    #[serde(default)]
    pub extension_settings: std::collections::BTreeMap<String, crate::workspace::ExtensionSettings>,
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
    pub channels: Vec<acheflow_extension_api::integration::Channel>,
    pub donations: Vec<acheflow_extension_api::integration::Donation>,
    pub interactions: Vec<acheflow_extension_api::integration::Interaction>,
    pub scenes: Vec<Scene>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlAction {
    ForwardExtension {
        url: String,
        instance_id: String,
    },
    SetClientCapabilities {
        grants: Vec<acheflow_extension_api::client_execution::CapabilityGrant>,
    },
    ExtensionEvent {
        id: String,
        action_id: String,
        value: String,
        #[serde(default)]
        config: Option<std::collections::BTreeMap<String, String>>,
    },
    SendChat {
        targets: Vec<String>,
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
        #[serde(default)]
        config: Option<std::collections::BTreeMap<String, String>>,
    },
    TriggerInteraction {
        id: String,
    },
    SetInteraction {
        interaction: acheflow_extension_api::integration::Interaction,
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
