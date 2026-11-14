use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
}
// Full configuration is returned only to a device with every Apply permission.
// No Debug: configurations can contain ingest and output keys.
#[derive(Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub server_id: String,
    pub revision: u64,
    pub summary: Option<WorkspaceSummary>,
    pub workspace: Option<Workspace>,
    /// Redacted display projection; never a source for Apply or account authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<Workspace>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportWorkspace {
    pub expected_revision: u64,
    pub workspace: Workspace,
}

/// Management-only comparison; never publish this secret-dependent hash to viewers.
pub fn fingerprint(workspace: &Workspace) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut config = workspace.server_configuration();
    config.migrate_spatial();
    config.edit_revision = 0;
    config.server_url.clear();
    let bytes =
        serde_json::to_vec(&config).map_err(|_| "Cannot compare workspace configuration.")?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
