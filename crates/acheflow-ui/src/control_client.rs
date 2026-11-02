use crate::project::Workspace;
use acheflow_media_api::control::{ControlAction, ControlRequest, ControlSnapshot};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
pub struct ControlClient {
    pub project: Signal<Workspace>,
    pub state: Signal<Option<ControlSnapshot>>,
    pub error: Signal<Option<String>>,
    pub busy: Signal<bool>,
}
pub fn use_control_client(project: Signal<Workspace>) -> ControlClient {
    let mut client = ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        busy: use_signal(|| false),
    };
    use_context_provider(|| client);
    let identity = use_memo(move || {
        let p = project.read();
        (p.id.clone(), p.server_url.clone())
    });
    use_resource(move || {
        let owner = identity();
        client.state.set(None);
        client.error.set(None);
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
    fn accept(mut self, snapshot: ControlSnapshot) {
        if snapshot.workspace_id.as_deref() == Some(self.project.peek().id.as_str()) {
            if self.project.peek().edit_revision != snapshot.edit_revision {
                self.project.write().edit_revision = snapshot.edit_revision;
            }
            let changed = self.state.peek().as_ref().is_some_and(|old| {
                old.workspace_id == snapshot.workspace_id && old.revision != snapshot.revision
            });
            if changed
                && let Some(previous) = self.state.peek().clone()
                && merge_remote_spatial(&mut self.project.write(), &previous, &snapshot)
            {
                self.error.set(Some(
                    "The scene changed remotely. Apply your draft or reload it before editing."
                        .into(),
                ));
            }
        }
        self.state.set(Some(snapshot));
    }
    pub fn command(mut self, action: ControlAction) {
        if *self.busy.peek() {
            return;
        }
        let workspace = self.project.peek().clone();
        let Some(state) = self
            .state
            .peek()
            .clone()
            .filter(|s| s.workspace_id.as_deref() == Some(workspace.id.as_str()))
        else {
            self.error
                .set(Some("Apply the workspace to its server first.".into()));
            return;
        };
        let request = ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:{}", timestamp(), crate::project::new_token()),
            expected_revision: state.revision,
            action,
        };
        self.busy.set(true);
        self.error.set(None);
        spawn(async move {
            let result = fetch(&workspace, Some(request)).await;
            if self.project.peek().id == workspace.id
                && self.project.peek().server_url == workspace.server_url
            {
                match result {
                    Ok(state) => self.accept(state),
                    Err(error) => self.error.set(Some(error)),
                }
            }
            self.busy.set(false);
        });
    }
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
