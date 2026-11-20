//! Acheflow permission for a device-local operation; no platform credentials.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeDirectAction {
    pub workspace_id: String,
    pub provider: String,
    pub channel_id: String,
    pub operation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectActionPermission {
    pub server_id: String,
    pub user_id: String,
    pub device_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub channel_id: String,
    pub server_time_ms: u64,
    pub valid_until_ms: u64,
}
