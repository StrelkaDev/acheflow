use crate::project::Workspace;
use acheflow_media_api::control::{ControlAction, ControlRequest, ControlSnapshot};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
pub struct ControlClient {
    pub project: Signal<Workspace>,
    pub state: Signal<Option<ControlSnapshot>>,
    pub error: Signal<Option<String>>,
    pub extension_errors: Signal<std::collections::BTreeMap<String, String>>,
    pub busy: Signal<bool>,
    pub sync_conflict: Signal<bool>,
}
pub fn use_control_client(project: Signal<Workspace>) -> ControlClient {
    let mut client = ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(std::collections::BTreeMap::new),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
    };
    use_context_provider(|| client);
    let identity = use_memo(move || {
        let p = project.read();
        (p.id.clone(), p.server_url.clone())
    });
    use_resource(move || {
        let owner = identity();
        client.state.set(None);
        client.sync_conflict.set(false);
        client.error.set(None);
        client.extension_errors.write().clear();
        async move {
            loop {
                if !*client.busy.peek() {
                    let workspace = project.peek().clone();
                    let result = fetch(&workspace, None).await;
                    if identity() != owner {
                        return;
                    }
                    match result {
                        Ok(state) => {
                            client.accept(state);
                        }
                        Err(error) => client.error.set(Some(error)),
                    }
                }
                crate::scene_preview::pause(1000).await;
            }
        }
    });
    client
}
impl ControlClient {
    pub(crate) fn accept(mut self, snapshot: ControlSnapshot) {
        let previous = self
            .state
            .peek()
            .clone()
            .filter(|old| old.workspace_id == snapshot.workspace_id);
        let owner = self.project.peek().id.clone();
        if snapshot.workspace_id.as_deref() == Some(owner.as_str()) {
            let mut workspace = self.project.write();
            let had_conflict = *self.sync_conflict.peek();
            let mut conflict = had_conflict;
            match previous {
                Some(previous) if previous.revision != snapshot.revision => {
                    conflict |= merge_remote_spatial(&mut workspace, &previous, &snapshot);
                    conflict |= merge_remote_extensions(&mut workspace, &previous, &snapshot);
                }
                None if workspace.edit_revision != snapshot.edit_revision => {
                    conflict |= !matches_remote_settings(&workspace, &snapshot);
                }
                _ => {}
            }
            if matches_remote_settings(&workspace, &snapshot) {
                conflict = false;
            }
            if !conflict {
                workspace.edit_revision = snapshot.edit_revision;
            }
            self.sync_conflict.set(conflict);
            if had_conflict && !conflict {
                self.error.set(None);
            }
            if conflict {
                self.error.set(Some("Workspace settings changed on the server. Your local edits are preserved; choose which settings to use.".into()));
            }
        } else {
            self.sync_conflict.set(false);
        }
        self.state.set(Some(snapshot));
    }
    /// Resolve against the last observed server state. Keeping local edits only
    /// acknowledges that revision; Apply remains a separate explicit action.
    pub fn resolve_sync_conflict(mut self, use_server: bool) {
        let Some(snapshot) = self
            .state
            .peek()
            .clone()
            .filter(|s| s.workspace_id.as_deref() == Some(self.project.peek().id.as_str()))
        else {
            return;
        };
        if !*self.sync_conflict.peek() {
            return;
        }
        let mut workspace = self.project.write();
        if use_server {
            workspace.extensions = snapshot.extension_settings;
            workspace.client_capabilities = snapshot.client_capabilities;
            for remote in snapshot.scenes {
                if let Some(local) = workspace.scenes.iter_mut().find(|s| s.id == remote.id) {
                    local.spatial = remote.spatial;
                }
            }
        }
        workspace.edit_revision = snapshot.edit_revision;
        self.sync_conflict.set(false);
        self.error.set(None);
    }
    pub fn set_extension(mut self, id: String, enabled: bool) {
        self.extension_errors.write().remove(&id);
        if self
            .state
            .peek()
            .as_ref()
            .is_some_and(|s| s.workspace_id.as_deref() == Some(self.project.peek().id.as_str()))
        {
            self.command(ControlAction::SetExtension {
                id,
                enabled,
                config: None,
            });
        } else {
            self.project
                .write()
                .extensions
                .entry(id)
                .or_default()
                .enabled = enabled;
        }
    }
    pub fn command(mut self, mut action: ControlAction) {
        if *self.busy.peek() {
            return;
        }
        match &mut action {
            ControlAction::ExtensionEvent { id, config, .. }
            | ControlAction::SetExtension { id, config, .. } => {
                *config = Some(
                    self.project
                        .peek()
                        .extensions
                        .get(id)
                        .map(|s| s.config.clone())
                        .unwrap_or_default(),
                );
            }
            _ => {}
        }
        let extension = match &action {
            ControlAction::SetExtension { id, .. } | ControlAction::ExtensionEvent { id, .. } => {
                Some(id.clone())
            }
            _ => None,
        };
        self.command_error(extension.as_deref(), None);
        let workspace = self.project.peek().clone();
        let Some(state) = self
            .state
            .peek()
            .clone()
            .filter(|s| s.workspace_id.as_deref() == Some(workspace.id.as_str()))
        else {
            self.command_error(
                extension.as_deref(),
                Some("Apply the workspace to its server first.".into()),
            );
            return;
        };
        let request = ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:{}", timestamp(), crate::project::new_token()),
            expected_revision: state.revision,
            action,
        };
        self.busy.set(true);
        spawn(async move {
            let result = fetch(&workspace, Some(request)).await;
            if self.project.peek().id == workspace.id
                && self.project.peek().server_url == workspace.server_url
            {
                match result {
                    Ok(state) => self.accept(state),
                    Err(error) => self.command_error(extension.as_deref(), Some(error)),
                }
            }
            self.busy.set(false);
        });
    }
    fn command_error(mut self, extension: Option<&str>, error: Option<String>) {
        if let Some(id) = extension {
            let mut errors = self.extension_errors.write();
            if let Some(error) = error {
                errors.insert(id.into(), error);
            } else {
                errors.remove(id);
            }
        } else {
            self.error.set(error);
        }
    }
}
fn matches_remote_settings(workspace: &Workspace, snapshot: &ControlSnapshot) -> bool {
    workspace.extensions == snapshot.extension_settings
        && workspace.client_capabilities == snapshot.client_capabilities
        && snapshot.scenes.iter().all(|remote| {
            workspace
                .scenes
                .iter()
                .any(|local| local.id == remote.id && local.spatial == remote.spatial)
        })
}
fn merge_remote_extensions(
    workspace: &mut Workspace,
    previous: &ControlSnapshot,
    next: &ControlSnapshot,
) -> bool {
    let ids = previous
        .extension_settings
        .keys()
        .chain(next.extension_settings.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let mut conflict = false;
    if previous.client_capabilities != next.client_capabilities {
        if workspace.client_capabilities == previous.client_capabilities
            || workspace.client_capabilities == next.client_capabilities
        {
            workspace.client_capabilities = next.client_capabilities.clone();
        } else {
            conflict = true;
        }
    }
    for id in ids {
        let base = previous
            .extension_settings
            .get(id)
            .cloned()
            .unwrap_or_default();
        let remote = next.extension_settings.get(id).cloned().unwrap_or_default();
        let local = workspace.extensions.entry(id.clone()).or_default();
        // Merge individual fields so an enable/disable response preserves a local client ID draft.
        if remote.enabled != base.enabled {
            local.enabled = remote.enabled;
        }
        for key in base
            .config
            .keys()
            .chain(remote.config.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let before = base.config.get(key);
            let after = remote.config.get(key);
            if before != after {
                if local.config.get(key) == before || local.config.get(key) == after {
                    if let Some(value) = after {
                        local.config.insert(key.clone(), value.clone());
                    } else {
                        local.config.remove(key);
                    }
                } else {
                    conflict = true;
                }
            }
        }
    }
    conflict
}
fn merge_remote_spatial(
    workspace: &mut Workspace,
    previous: &ControlSnapshot,
    next: &ControlSnapshot,
) -> bool {
    let mut conflict = false;
    for remote in &next.scenes {
        let Some(local) = workspace.scenes.iter_mut().find(|s| s.id == remote.id) else {
            continue;
        };
        let Some(base) = previous.scenes.iter().find(|s| s.id == remote.id) else {
            continue;
        };
        if remote.spatial == base.spatial {
            continue;
        }
        if local.spatial != base.spatial && local.spatial != remote.spatial {
            conflict = true;
            continue;
        }
        local.spatial = remote.spatial.clone();
    }
    conflict
}

fn timestamp() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
}
async fn fetch(
    workspace: &Workspace,
    command: Option<ControlRequest>,
) -> Result<ControlSnapshot, String> {
    let path = if command.is_some() {
        "/api/control/commands"
    } else {
        "/api/control/state"
    };
    let endpoint = crate::media::server_url(workspace, path)?;
    let http = reqwest::Client::new();
    let request = match command {
        Some(command) => http.post(endpoint).json(&command),
        None => http.get(endpoint),
    };
    let response = request
        .send()
        .await
        .map_err(|_| "Could not reach the interaction service".to_owned())?;
    if !response.status().is_success() {
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Interaction command failed")
            .to_owned());
    }
    response
        .json()
        .await
        .map_err(|_| "Invalid interaction service response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_capability_policy_is_preserved_without_overwriting_a_local_draft() {
        use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
        let mut workspace = Workspace::default();
        let previous = ControlSnapshot::default();
        let mut next = previous.clone();
        next.client_capabilities = vec![CapabilityGrant {
            provider: "fixture".into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }];
        assert!(!merge_remote_extensions(&mut workspace, &previous, &next));
        assert_eq!(workspace.client_capabilities, next.client_capabilities);
        let previous = next.clone();
        workspace.client_capabilities[0]
            .capabilities
            .insert(Capability::ChatSend);
        next.client_capabilities.clear();
        assert!(merge_remote_extensions(&mut workspace, &previous, &next));
        assert!(
            workspace.client_capabilities[0]
                .capabilities
                .contains(&Capability::ChatSend)
        );
    }
    #[test]
    fn remote_toggles_preserve_public_client_id_drafts_and_report_conflicting_edits() {
        let mut workspace = Workspace::default();
        workspace
            .extensions
            .entry("twitch".into())
            .or_default()
            .config
            .insert("application".into(), "localdraft".into());
        let previous = ControlSnapshot::default();
        let mut next = previous.clone();
        next.extension_settings
            .entry("twitch".into())
            .or_default()
            .enabled = true;
        assert!(!merge_remote_extensions(&mut workspace, &previous, &next));
        assert!(workspace.extensions["twitch"].enabled);
        assert_eq!(
            workspace.extensions["twitch"].config["application"],
            "localdraft"
        );
        let previous = next.clone();
        next.extension_settings.get_mut("twitch").unwrap().enabled = false;
        next.extension_settings
            .get_mut("twitch")
            .unwrap()
            .config
            .insert("application".into(), "remotedraft".into());
        assert!(merge_remote_extensions(&mut workspace, &previous, &next));
        assert!(!workspace.extensions["twitch"].enabled);
        assert_eq!(
            workspace.extensions["twitch"].config["application"],
            "localdraft"
        );
    }
}
