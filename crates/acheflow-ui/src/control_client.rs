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
    pub(crate) request_generation: Signal<u64>,
}
pub fn use_control_client(project: Signal<Workspace>) -> ControlClient {
    crate::client_authorization::provide_accounts(project);
    let mut client = ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(std::collections::BTreeMap::new),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
        request_generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    let identity = use_memo(move || {
        let p = project.read();
        (
            p.id.clone(),
            p.server_url.clone(),
            crate::server_access::identity(),
        )
    });
    use_resource(move || {
        let owner = identity();
        client.state.set(None);
        if try_consume_context::<crate::workspace_sync::WorkspaceSync>().is_none() {
            client.sync_conflict.set(false);
        }
        client.error.set(None);
        client.extension_errors.write().clear();
        client.busy.set(false);
        *client.request_generation.write() += 1;
        async move {
            let mut failures = 0u8;
            loop {
                if !*client.busy.peek() {
                    let generation = *client.request_generation.peek();
                    let workspace = project.peek().clone();
                    let result = fetch(&workspace, None).await;
                    if identity() != owner {
                        return;
                    }
                    if !client.response_current(generation, &workspace, &owner.2)
                        || *client.busy.peek()
                    {
                        continue;
                    }
                    match result {
                        Ok(state) => {
                            failures = 0;
                            client.accept(state);
                        }
                        Err(error) => {
                            failures = failures.saturating_add(1);
                            if client.error.peek().as_ref() != Some(&error) {
                                client.error.set(Some(error));
                            }
                        }
                    }
                }
                // Platform chats can remain active while the media stream is stopped.
                let active = client.state.peek().as_ref().is_some_and(|s| {
                    s.workspace_id.as_deref() == Some(owner.0.as_str()) && !s.channels.is_empty()
                });
                crate::scene_preview::pause(crate::live_client::polling_delay(
                    active, failures, 1000,
                ))
                .await;
            }
        }
    });
    client
}
impl ControlClient {
    pub(crate) fn response_current(
        &self,
        generation: u64,
        workspace: &Workspace,
        identity: &Option<String>,
    ) -> bool {
        generation == *self.request_generation.peek()
            && self.project.peek().id == workspace.id
            && self.project.peek().server_url == workspace.server_url
            && crate::server_access::identity() == *identity
    }
    pub(crate) fn accept(mut self, snapshot: ControlSnapshot) {
        if try_consume_context::<crate::workspace_sync::WorkspaceSync>().is_some() {
            self.state.set(Some(snapshot));
            return;
        }
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
            if let Some(fingerprint) = snapshot.workspace_fingerprint.as_ref() {
                if acheflow_media_api::server_workspace::fingerprint(&workspace)
                    .as_ref()
                    .ok()
                    == Some(fingerprint)
                {
                    conflict = false;
                    workspace.edit_revision = snapshot.edit_revision;
                } else if workspace.edit_revision != snapshot.edit_revision {
                    conflict = true;
                }
            }
            // A partial/read-only snapshot cannot acknowledge unseen media edits.
            self.sync_conflict.set(conflict);
            if had_conflict && !conflict {
                self.error.set(None);
            }
            if conflict {
                self.error.set(Some("Workspace changed on the server. Your local edits are preserved; choose which workspace configuration to use.".into()));
            }
        } else {
            self.sync_conflict.set(false);
        }
        self.state.set(Some(snapshot));
    }
    /// Resolve against the last observed server state. Keeping local edits only
    /// acknowledges that revision; Apply remains a separate explicit action.
    pub fn resolve_sync_conflict(mut self, use_server: bool) {
        if let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>() {
            sync.resolve(use_server);
            return;
        }
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
        if !use_server {
            self.project.write().edit_revision = snapshot.edit_revision;
            self.sync_conflict.set(false);
            self.error.set(None);
            return;
        }
        let owner = self.project.peek().clone();
        self.busy.set(true);
        spawn(async move {
            let result = crate::server_workspace::fetch(&owner).await;
            if self.project.peek().id != owner.id
                || self.project.peek().server_url != owner.server_url
            {
                self.busy.set(false);
                return;
            }
            match result {
                Ok(state) => {
                    if let Some(mut remote) = state.workspace.filter(|w| w.id == owner.id) {
                        remote.server_url = owner.server_url;
                        remote.client_extensions = self.project.peek().client_extensions.clone();
                        self.project.set(remote);
                        self.sync_conflict.set(false);
                        self.error.set(None);
                    } else {
                        self.error.set(Some("The server workspace changed or this device cannot load its configuration.".into()));
                    }
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
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
        if *self.sync_conflict.peek()
            && matches!(
                &action,
                ControlAction::SetScene { .. }
                    | ControlAction::SetExtension { .. }
                    | ControlAction::SetClientCapabilities { .. }
                    | ControlAction::SetCallbackProxy { .. }
                    | ControlAction::ExtensionEvent { .. }
            )
        {
            self.error.set(Some(
                "Resolve the workspace conflict before changing its configuration.".into(),
            ));
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
        *self.request_generation.write() += 1;
        let generation = *self.request_generation.peek();
        let identity = crate::server_access::identity();
        spawn(async move {
            let result = fetch(&workspace, Some(request)).await;
            if !self.response_current(generation, &workspace, &identity) {
                return;
            }
            match result {
                Ok(state) => self.accept(state),
                Err(error) => self.command_error(extension.as_deref(), Some(error)),
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
pub(crate) async fn fetch(
    workspace: &Workspace,
    command: Option<ControlRequest>,
) -> Result<ControlSnapshot, String> {
    let path = if command.is_some() {
        "/api/control/commands"
    } else {
        "/api/control/state"
    };
    let endpoint = crate::media::server_url(workspace, path)?;
    let request = match command {
        Some(command) => {
            crate::server_access::request(reqwest::Method::POST, &endpoint)?.json(&command)
        }
        None => crate::server_access::request(reqwest::Method::GET, &endpoint)?,
    };
    let response = request
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| format!("Could not reach the interaction service at {endpoint}. Check the server address and that the server is running."))?;
    if !response.status().is_success() {
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(value["error"]
            .as_str()
            .or_else(|| value["error"]["message"].as_str())
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
