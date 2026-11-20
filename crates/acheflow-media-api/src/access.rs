//! Acheflow device permissions; independent of provider accounts and OAuth scopes.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    StudioView,
    InteractiveView,
    ChatRead,
    ChatSend,
    ScenesManage,
    MediaManage,
    WorkspaceManage,
    InteractiveManage,
    ExtensionsConfigure,
    ExtensionsRun,
    ExtensionsUpload,
    ExtensionsInstall,
    AccessManage,
}
impl Permission {
    pub const ALL: [Self; 13] = [
        Self::StudioView,
        Self::InteractiveView,
        Self::ChatRead,
        Self::ChatSend,
        Self::ScenesManage,
        Self::MediaManage,
        Self::WorkspaceManage,
        Self::InteractiveManage,
        Self::ExtensionsConfigure,
        Self::ExtensionsRun,
        Self::ExtensionsUpload,
        Self::ExtensionsInstall,
        Self::AccessManage,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::StudioView => "View studio",
            Self::InteractiveView => "View interactive",
            Self::ChatRead => "Read chat",
            Self::ChatSend => "Send chat",
            Self::ScenesManage => "Control scenes",
            Self::MediaManage => "Start / stop media",
            Self::WorkspaceManage => "Apply workspace configuration",
            Self::InteractiveManage => "Control interactive",
            Self::ExtensionsConfigure => "Configure extensions",
            Self::ExtensionsRun => "Run extensions",
            Self::ExtensionsUpload => "Upload extension packages",
            Self::ExtensionsInstall => "Install extensions",
            Self::AccessManage => "Manage device access",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub user_id: String,
    pub id: String,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub user: User,
    pub server_id: String,
    pub device: Device,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRequest {
    pub code: String,
    pub name: String,
}
// Deliberately no Debug on DTOs carrying bearer secrets or invitation codes.
#[derive(Serialize, Deserialize)]
pub struct PairResponse {
    pub session: Session,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRequest {
    pub permissions: BTreeSet<Permission>,
}
#[derive(Serialize, Deserialize)]
pub struct Invitation {
    pub code: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Devices {
    pub devices: Vec<Device>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequest {
    pub name: String,
}
