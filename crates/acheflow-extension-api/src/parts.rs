//! Package parts select authority before a host opens code, settings or secrets.
use crate::integration::{ExecutionSide, GuestManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PARTS_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionHost {
    #[default]
    Client,
    Server,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Logic,
    Provider,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionParts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ExtensionPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ExtensionPart>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionPart {
    pub kind: PartKind,
    pub module: String,
    #[serde(default = "default_entry_point")]
    pub entry_point: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub supported_actions: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub secret_fields: BTreeMap<String, String>,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(default)]
    pub settings: Vec<crate::integration::Setting>,
    #[serde(default)]
    pub unique_viewers: bool,
    #[serde(default)]
    pub authentication: Option<serde_json::Value>,
    #[serde(default)]
    pub application_setup: Option<crate::application_setup::ApplicationSetup>,
    #[serde(default)]
    pub forwarding: Vec<crate::forwarding::ForwardAction>,
    #[serde(default)]
    pub source_types: Vec<crate::visual::SourceType>,
    #[serde(default)]
    pub chat_presentation: Option<crate::integration::ChatPresentation>,
}
pub(crate) fn default_entry_point() -> String {
    "handle".into()
}
pub fn valid_entry_point(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
impl ExtensionPart {
    fn resolve(
        &self,
        package: &GuestManifest,
        host: ExecutionHost,
    ) -> Result<GuestManifest, String> {
        if host == ExecutionHost::Client && self.kind == PartKind::Logic {
            return Err("client parts must use the provider interface".into());
        }
        let mut value = serde_json::to_value(self).map_err(|_| "invalid extension part")?;
        let object = value.as_object_mut().ok_or("invalid extension part")?;
        object.remove("kind");
        object.insert("id".into(), package.id.clone().into());
        object.insert("name".into(), package.name.clone().into());
        object.insert("api_version".into(), package.api_version.into());
        // This legacy field describes the guest interface, not the selected host.
        object.insert(
            "execution_side".into(),
            match self.kind {
                PartKind::Logic => "server",
                PartKind::Provider => "client",
            }
            .into(),
        );
        let mut manifest: GuestManifest =
            serde_json::from_value(value).map_err(|e| format!("invalid extension part: {e}"))?;
        manifest.selected_host = Some(host);
        manifest.validate()?;
        Ok(manifest)
    }
}
impl GuestManifest {
    /// Returns only the selected host's declarations. Other part settings/secrets
    /// are not inherited. None means this package does not implement that host.
    pub fn select_part(&self, host: ExecutionHost) -> Result<Option<Self>, String> {
        self.validate()?;
        if let Some(parts) = &self.parts {
            let part = match host {
                ExecutionHost::Client => &parts.client,
                ExecutionHost::Server => &parts.server,
            };
            return part.as_ref().map(|p| p.resolve(self, host)).transpose();
        }
        if self.selected_host.is_some_and(|selected| selected != host) {
            return Err("extension part belongs to another host".into());
        }
        if host == ExecutionHost::Client && self.execution_side == ExecutionSide::Server {
            return Ok(None);
        }
        // Legacy client manifests remain eligible for the explicit trusted server
        // loader during migration. New packages require an actual server part.
        let mut selected = self.clone();
        selected.selected_host = Some(host);
        Ok(Some(selected))
    }
    pub fn selected_host(&self) -> Option<ExecutionHost> {
        self.selected_host
    }
    pub fn require_host(&self, host: ExecutionHost) -> Result<(), String> {
        if self.parts.is_some() || self.selected_host.is_some_and(|selected| selected != host) {
            return Err("select the extension part for the executing host before loading".into());
        }
        Ok(())
    }
}
pub(crate) fn validate_package(package: &GuestManifest) -> Result<(), String> {
    if package.parts_version != Some(PARTS_VERSION) {
        return Err("unsupported extension parts version; expected parts_version 1".into());
    }
    let root_declarations = [
        ("module", !package.module.is_empty()),
        ("entry_point", package.entry_point != "handle"),
        (
            "execution_side",
            package.execution_side != ExecutionSide::Server,
        ),
        ("supported_actions", package.supported_actions.is_some()),
        ("capabilities", !package.capabilities.is_empty()),
        ("services", !package.services.is_empty()),
        ("secret_fields", !package.secret_fields.is_empty()),
        ("subscriptions", !package.subscriptions.is_empty()),
        ("settings", !package.settings.is_empty()),
        ("unique_viewers", package.unique_viewers),
        ("authentication", package.authentication.is_some()),
        ("application_setup", package.application_setup.is_some()),
        ("forwarding", !package.forwarding.is_empty()),
        ("source_types", !package.source_types.is_empty()),
        ("chat_presentation", package.chat_presentation.is_some()),
    ];
    if let Some((field, _)) = root_declarations.iter().find(|(_, declared)| *declared) {
        return Err(format!(
            "part packages cannot mix legacy root runtime declaration '{field}' with parts"
        ));
    }
    let parts = package
        .parts
        .as_ref()
        .ok_or("extension parts are missing")?;
    if parts.client.is_none() && parts.server.is_none() {
        return Err("declare at least one extension part".into());
    }
    for (host, part) in [
        (ExecutionHost::Client, &parts.client),
        (ExecutionHost::Server, &parts.server),
    ] {
        if let Some(part) = part {
            part.resolve(package, host)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_actions(actions: Option<&[String]>) -> Result<(), String> {
    let Some(actions) = actions else {
        return Ok(());
    };
    let unique: std::collections::BTreeSet<_> = actions.iter().collect();
    if actions.len() > 16 || unique.len() != actions.len() {
        return Err("supported actions must be unique and bounded".into());
    }
    if actions.iter().any(|action| {
        !matches!(
            action.as_str(),
            "chat.send" | "chat.delete" | "user.timeout" | "poll.create" | "poll.close"
        )
    }) {
        return Err("invalid supported action declaration".into());
    }
    Ok(())
}
impl GuestManifest {
    /// Legacy manifests preserve existing domain/permission checks. Explicit
    /// parts can only narrow the set of platform operations implemented here.
    pub fn allows_action(&self, action: &str) -> bool {
        self.supported_actions
            .as_ref()
            .is_none_or(|actions| actions.iter().any(|allowed| allowed == action))
    }
}
