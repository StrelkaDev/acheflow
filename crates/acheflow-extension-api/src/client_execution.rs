//! Credential-free domain contract between a client executor and the logic server.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const CLIENT_EXECUTION_VERSION: u32 = 1;
pub const REGISTRATION_TTL_MS: u64 = 30_000;
pub const INTENT_RETENTION_MS: u64 = 600_000;
pub const MAX_INSTRUCTION_LIFETIME_MS: u64 = 300_000;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "chat.read")]
    ChatRead,
    #[serde(rename = "chat.send")]
    ChatSend,
    #[serde(rename = "audience.read")]
    AudienceRead,
    #[serde(rename = "chat.delete")]
    ChatDelete,
    #[serde(rename = "user.timeout")]
    UserTimeout,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub provider: String,
    pub account_id: String,
    pub channel_id: String,
    pub capabilities: BTreeSet<Capability>,
}

/// Saved workspace policy; client registration can only narrow these grants.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityGrant {
    pub provider: String,
    pub account_id: String,
    pub channel_id: String,
    pub capabilities: BTreeSet<Capability>,
}

/// Actor/workspace identity is supplied by the authenticated host, never this body.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRegistration {
    pub version: u32,
    pub generation: u64,
    pub connections: Vec<Connection>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorStatus {
    pub client_id: String,
    pub generation: u64,
    pub expires_at_ms: u64,
    pub connections: Vec<Connection>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientAction {
    SendChat {
        text: String,
    },
    DeleteChat {
        message_id: String,
    },
    TimeoutUser {
        user_id: String,
        duration_seconds: u32,
    },
}
impl ClientAction {
    pub fn capability(&self) -> Capability {
        match self {
            Self::SendChat { .. } => Capability::ChatSend,
            Self::DeleteChat { .. } => Capability::ChatDelete,
            Self::TimeoutUser { .. } => Capability::UserTimeout,
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionState {
    Queued,
    InFlight,
    Sent,
    Failed,
    Rejected,
    Unknown,
    Cancelled,
    Expired,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionRequest {
    pub id: String,
    #[serde(default)]
    pub request_id: Option<String>,
    pub client_id: String,
    pub generation: u64,
    pub connection_id: String,
    pub action: ClientAction,
    pub expires_at_ms: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub connection: Connection,
    pub id: String,
    #[serde(default)]
    pub request_id: Option<String>,
    pub workspace_id: String,
    pub client_id: String,
    pub generation: u64,
    pub connection_id: String,
    pub action: ClientAction,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub state: InstructionState,
    pub external_id: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedInstruction {
    pub instruction: Instruction,
    pub lease: u64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Sent,
    Failed,
    Rejected,
    Unknown,
}
impl From<ExecutionOutcome> for InstructionState {
    fn from(value: ExecutionOutcome) -> Self {
        match value {
            ExecutionOutcome::Sent => Self::Sent,
            ExecutionOutcome::Failed => Self::Failed,
            ExecutionOutcome::Rejected => Self::Rejected,
            ExecutionOutcome::Unknown => Self::Unknown,
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub instruction_id: String,
    pub lease: u64,
    pub outcome: ExecutionOutcome,
    pub external_id: Option<String>,
}

/// Only normalized public domain data crosses this boundary. No arbitrary JSON effects.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientEvent {
    Chat {
        message: crate::interaction::ChatMessage,
    },
    Audience {
        sample: crate::audience::AudienceSample,
    },
    Delete {
        message_id: String,
    },
    ClearChat {},
}
impl ClientEvent {
    pub fn capability(&self) -> Capability {
        match self {
            Self::Chat { .. } | Self::Delete { .. } | Self::ClearChat { .. } => {
                Capability::ChatRead
            }
            Self::Audience { .. } => Capability::AudienceRead,
        }
    }
}

/// Workspace and actor are pinned by the authenticated MCP endpoint.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorRequest {
    Register {
        registration: ClientRegistration,
    },
    Unregister {
        generation: u64,
    },
    Publish {
        generation: u64,
        connection_id: String,
        event: ClientEvent,
    },
    Claim {
        generation: u64,
        limit: usize,
    },
    Report {
        generation: u64,
        receipt: ExecutionReceipt,
    },
}
