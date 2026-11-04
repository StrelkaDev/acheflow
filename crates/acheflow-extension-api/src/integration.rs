//! Provider-neutral extension/domain contracts. See docs/extension-contract.md.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub help: String,
    #[serde(default = "setting_limit")]
    pub max_length: usize,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub toggle: bool,
}
impl Default for Setting {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            help: String::new(),
            max_length: setting_limit(),
            secret: false,
            toggle: false,
        }
    }
}
fn setting_limit() -> usize {
    8192
}
/// Execution authority is explicit; server logic never receives provider I/O.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSide {
    #[default]
    Server,
    Client,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestManifest {
    pub id: String,
    pub name: String,
    pub api_version: u32,
    pub module: String,
    #[serde(default)]
    pub execution_side: ExecutionSide,
    pub capabilities: Vec<String>,
    pub services: Vec<String>,
    #[serde(default)]
    pub secret_fields: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(default)]
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub unique_viewers: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_setup: Option<crate::application_setup::ApplicationSetup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwarding: Vec<crate::forwarding::ForwardAction>,
}
impl GuestManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.api_version != crate::API_VERSION {
            return Err("unsupported extension API version".into());
        }
        if self.execution_side == ExecutionSide::Server
            && (!self.services.is_empty()
                || self.authentication.is_some()
                || !self.secret_fields.is_empty()
                || self.settings.iter().any(|s| s.secret)
                || self.capabilities.iter().any(|c| client_only_capability(c)))
        {
            return Err(
                "server logic extensions cannot request provider transport or credentials".into(),
            );
        }
        if matches!(self.id.as_str(), "acheflow" | "ui")
            || !valid_id(&self.id, 64)
            || self.id.contains([':', '.'])
            || self.name.is_empty()
            || self.name.len() > 128
        {
            return Err("invalid extension identity".into());
        }
        if self.module.is_empty() || self.module.contains(['/', '\\']) || self.module.contains("..")
        {
            return Err("extension module must be a local file name".into());
        }
        if self.capabilities.len() > 24
            || self.services.len() > 16
            || self.settings.len() > 32
            || self.subscriptions.len() > 32
            || self.secret_fields.len() > 32
        {
            return Err("extension manifest limit reached".into());
        }
        for capability in &self.capabilities {
            if !client_only_capability(capability)
                && !matches!(
                    capability.as_str(),
                    "chat"
                        | "audience"
                        | "delete"
                        | "delivery"
                        | "failure"
                        | "panel"
                        | "scene"
                        | "channel"
                        | "poll"
                        | "donation"
                        | "interaction"
                        | "status"
                        | "storage"
                        | "read.chat"
                        | "read.audience"
                        | "read.poll"
                        | "read.donation"
                        | "read.interaction"
                )
            {
                return Err("unknown extension capability".into());
            }
        }
        if self
            .secret_fields
            .iter()
            .any(|(key, path)| !valid_id(key, 64) || !path.starts_with('/') || path.len() > 256)
        {
            return Err("invalid secret capture declaration".into());
        }
        self.validate_forwarding()?;
        let mut ids = std::collections::BTreeSet::new();
        for setting in &self.settings {
            if !valid_id(&setting.id, 64)
                || !ids.insert(&setting.id)
                || setting.label.len() > 128
                || setting.help.len() > 1024
                || !(1..=8192).contains(&setting.max_length)
            {
                return Err("invalid extension setting".into());
            }
        }
        self.validate_application_setup()?;
        for topic in &self.subscriptions {
            if !matches!(
                topic.as_str(),
                "chat.send"
                    | "poll.create"
                    | "poll.close"
                    | "interaction.triggered"
                    | "interaction.registered"
                    | "chat.message"
                    | "donation.received"
                    | "audience.updated"
                    | "poll.updated"
                    | "chat.deleted"
                    | "chat.delivery"
                    | "channel.updated"
            ) {
                return Err("unknown extension subscription".into());
            }
            let read = match topic.as_str() {
                "chat.message" | "chat.deleted" | "chat.delivery" => Some("read.chat"),
                "audience.updated" => Some("read.audience"),
                "poll.updated" => Some("read.poll"),
                "donation.received" => Some("read.donation"),
                "interaction.triggered" | "interaction.registered" => Some("read.interaction"),
                _ => None,
            };
            if read.is_some_and(|cap| !self.capabilities.iter().any(|c| c == cap)) {
                return Err("subscription read capability is not granted".into());
            }
        }
        Ok(())
    }
    fn validate_forwarding(&self) -> Result<(), String> {
        use crate::forwarding::{ForwardMode, valid_forward_action};
        if self.forwarding.len() > 16 {
            return Err("extension manifest limit reached".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for action in &self.forwarding {
            if !valid_forward_action(&action.id) || !ids.insert(&action.id) {
                return Err("invalid or duplicate forwarding action".into());
            }
            if action.mode != ForwardMode::Callback {
                continue;
            }
            if self.execution_side != ExecutionSide::Client {
                return Err("callback forwarding belongs to client extensions".into());
            }
            if !["crypto", "forward_wait"]
                .iter()
                .all(|required| self.capabilities.iter().any(|c| c == required))
            {
                return Err(
                    "callback forwarding requires crypto and forward_wait capabilities".into(),
                );
            }
        }
        Ok(())
    }
}
fn client_only_capability(capability: &str) -> bool {
    matches!(
        capability,
        "http"
            | "websocket"
            | "crypto"
            | "browser"
            | "browser_module"
            | "secret_clear"
            | "secret_begin"
            | "secret_commit"
            | "secret_discard"
            | "forward_wait"
    )
}

pub fn valid_provider_id(id: &str) -> bool {
    valid_id(id, 64) && !id.contains([':', '.']) && !matches!(id, "acheflow" | "ui")
}
pub fn valid_id(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.len() <= max
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub extension_id: String,
    pub label: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub constraints: std::collections::BTreeMap<String, u64>,
}
impl Channel {
    pub fn key(&self) -> String {
        format!("{}:{}", self.extension_id, self.id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub channel: String,
    pub state: String,
    pub error: Option<String>,
    pub external_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollTarget {
    pub channel: String,
    pub external_id: Option<String>,
    pub state: String,
    pub votes: Vec<u64>,
    pub error: Option<String>,
    pub updated_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollUpdate {
    pub poll_id: u64,
    pub channel_id: String,
    pub external_id: Option<String>,
    pub state: String,
    pub votes: Vec<u64>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Donation {
    pub id: String,
    #[serde(default)]
    pub extension_id: String,
    pub channel_id: String,
    pub display_name: String,
    pub amount_minor: u64,
    pub currency: String,
    #[serde(default = "two")]
    pub decimal_places: u8,
    pub message: String,
    pub occurred_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneAction {
    pub scene_id: u32,
    pub object_id: u64,
    pub rotation: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interaction {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub action: Option<SceneAction>,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub api_version: u32,
    pub workspace_id: String,
    pub extension_id: String,
    pub instance_id: String,
    pub request_id: String,
    pub now_ms: u64,
    #[serde(flatten)]
    pub payload: Value,
}

fn two() -> u8 {
    2
}
