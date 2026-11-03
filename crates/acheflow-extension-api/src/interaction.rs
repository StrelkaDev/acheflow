use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub platform: String,
    pub channel_id: String,
    pub message_id: String,
    pub user_id: String,
    pub display_name: String,
    pub text: String,
    pub roles: Vec<String>,
    pub occurred_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEntry {
    pub cursor: u64,
    pub message: ChatMessage,
    pub deleted: bool,
    #[serde(default)]
    pub merged_into: Option<String>,
    #[serde(default)]
    pub delivery: Vec<crate::integration::Delivery>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatPage {
    pub entries: Vec<ChatEntry>,
    pub cursor: u64,
    pub gap: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollRequest {
    pub question: String,
    pub options: Vec<String>,
    pub channels: Vec<String>,
    pub duration_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub id: u64,
    pub question: String,
    pub options: Vec<String>,
    pub channels: Vec<String>,
    pub votes: Vec<u64>,
    pub closes_at_ms: u64,
    pub closed: bool,
    pub cancelled: bool,
    #[serde(default)]
    pub targets: Vec<crate::integration::PollTarget>,
}

pub fn channel_key(platform: &str, channel: &str) -> String {
    format!("{platform}:{channel}")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRule {
    pub name: String,
    pub channel: String,
    pub roles: Vec<String>,
    pub cooldown_ms: u64,
    pub scene_id: u32,
    pub object_id: u64,
    pub rotation: [f32; 3],
    pub enabled: bool,
}
