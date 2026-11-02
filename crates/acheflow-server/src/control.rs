//! Shared application services for the UI, extensions and MCP adapters.
use crate::live::RuntimeState;
use acheflow_control::{analytics::Analytics, interaction::Interaction};
use acheflow_media_api::control::*;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tokio_util::sync::CancellationToken;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
struct RequestResult {
    id: String,
    hash: [u8; 32],
    at: u64,
    result: Option<Result<(), String>>,
}
#[derive(Default)]
pub struct Services {
    pub panels: std::collections::BTreeMap<String, acheflow_extension_api::ExtensionPanel>,
    pub owner: Option<String>,
    pub analytics: Analytics,
    pub interaction: Interaction,
    pub extensions: Vec<ExtensionStatus>,
    pub authorization: Option<AuthorizationStatus>,
    requests: VecDeque<RequestResult>,
    expired_before: u64,
}

pub struct ControlHost {
    pub runtime: RuntimeState,
    pub services: Mutex<Services>,
    pub credentials: RwLock<Option<crate::twitch_auth::Credentials>>,
    pub twitch: AsyncMutex<Option<crate::extension_host::Running>>,
    pub auth_cancel: Mutex<Option<CancellationToken>>,
    command_lock: AsyncMutex<()>,
    pub installed: std::collections::BTreeMap<String, crate::extension_registry::Installed>,
    pub guests: AsyncMutex<std::collections::HashMap<String, crate::guest_host::Running>>,
    pub events: tokio::sync::broadcast::Sender<Value>,
}
impl ControlHost {
    pub fn new(runtime: RuntimeState) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            services: Mutex::new(Services::default()),
            credentials: RwLock::new(None),
            twitch: AsyncMutex::new(None),
            auth_cancel: Mutex::new(None),
            command_lock: AsyncMutex::new(()),
            installed: crate::extension_registry::discover(),
            guests: AsyncMutex::new(std::collections::HashMap::new()),
            events: tokio::sync::broadcast::channel(512).0,
        })
    }
    pub fn snapshot(&self, after: u64, limit: usize) -> ControlSnapshot {
        let workspace = self.runtime.workspace.read().unwrap();
        let owner = workspace.as_ref().map(|w| w.id.clone());
        let mut state = self.services.lock().unwrap();
        if state.owner != owner || state.extensions.is_empty() {
            *state = Services {
                owner: owner.clone(),
                extensions: self
                    .installed
                    .values()
                    .map(|e| ExtensionStatus {
                        id: e.manifest.id.clone(),
                        name: e.manifest.name.clone(),
                        state: "stopped".into(),
                        capabilities: e.manifest.capabilities.clone(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
        }
        ControlSnapshot {
            panels: state.panels.values().cloned().collect(),
            edit_revision: workspace.as_ref().map(|w| w.edit_revision).unwrap_or(0),
            workspace_id: owner,
            revision: self
                .runtime
                .control_revision
                .load(std::sync::atomic::Ordering::Acquire),
            chat: state.interaction.messages(after, limit),
            audience: state.analytics.snapshot(now_ms()),
            history: state.analytics.history().iter().cloned().collect(),
            polls: state.interaction.polls(now_ms()),
            commands: state.interaction.commands(),
            extensions: state.extensions.clone(),
            authorization: state.authorization.clone(),
            scenes: workspace
                .as_ref()
                .map(|w| w.scenes.clone())
                .unwrap_or_default(),
        }
    }
    pub fn owns(&self, id: &str) -> bool {
        self.runtime
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|w| w.id == id)
    }
    pub fn extension_status(&self, owner: &str, status: ExtensionStatus) {
        if !self.owns(owner) {
            return;
        }
        let mut state = self.services.lock().unwrap();
        if state.owner.as_deref() != Some(owner) {
            return;
        }
        if let Some(old) = state.extensions.iter_mut().find(|s| s.id == status.id) {
            *old = status;
        } else if state.extensions.len() < 8 {
            state.extensions.push(status);
        }
    }
    pub async fn publish(&self, owner: &str, effect: Value) -> Result<(), String> {
        self.publish_from(owner, "twitch", effect, 0).await
    }
    pub async fn publish_from(
        &self,
        owner: &str,
        origin: &str,
        effect: Value,
        depth: u8,
    ) -> Result<(), String> {
        let _control = if effect["type"] == "chat" {
            Some(self.runtime.control.lock().await)
        } else {
            None
        };
        self.publish_inner(owner, origin, effect, depth)
    }
    fn publish_inner(
        &self,
        owner: &str,
        origin: &str,
        effect: Value,
        depth: u8,
    ) -> Result<(), String> {
        if depth > 4 {
            return Err("extension event chain limit reached".into());
        }
        if !self.owns(owner) {
            return Err("workspace changed".into());
        }
        let mut state = self.services.lock().unwrap();
        if state.owner.as_deref() != Some(owner) {
            return Err("workspace changed".into());
        }
        match effect["type"].as_str() {
            Some("audience") => {
                let sample: acheflow_extension_api::audience::AudienceSample =
                    serde_json::from_value(effect["sample"].clone())
                        .map_err(|_| "invalid audience effect")?;
                if sample.platform != origin {
                    return Err(
                        "forbidden: extension cannot publish another platform's audience".into(),
                    );
                }
                state.analytics.record(sample)?;
                state.analytics.capture(now_ms());
            }
            Some("chat") => {
                let message: acheflow_extension_api::interaction::ChatMessage =
                    serde_json::from_value(effect["message"].clone())
                        .map_err(|_| "invalid chat effect")?;
                if message.platform != origin {
                    return Err(
                        "forbidden: extension cannot publish another platform's chat".into(),
                    );
                }
                let before = state.interaction.cursor();
                let action = state.interaction.ingest(message, now_ms())?;
                if state.interaction.cursor() == before {
                    return Ok(());
                }
                drop(state);
                if let Some(action) = action {
                    let mut workspace = self.runtime.workspace.write().unwrap();
                    let workspace = workspace
                        .as_mut()
                        .filter(|w| w.id == owner)
                        .ok_or("workspace changed")?;
                    let scene = workspace
                        .scenes
                        .iter_mut()
                        .find(|s| s.id == action.scene_id)
                        .ok_or("command scene not found")?;
                    let object = scene
                        .spatial
                        .as_mut()
                        .and_then(|s| s.objects.iter_mut().find(|o| o.id == action.object_id))
                        .ok_or("command object not found")?;
                    object.transform.rotation = action.rotation;
                    workspace.edit_revision = self
                        .runtime
                        .control_revision
                        .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                        + 1;
                }
            }
            Some("delete") => {
                let platform = effect["platform"].as_str().ok_or("missing platform")?;
                let channel = effect["channel_id"].as_str().ok_or("missing channel")?;
                if platform != origin {
                    return Err("forbidden: extension cannot delete another platform's chat".into());
                }
                let message_id = if effect["all"] == true {
                    None
                } else {
                    Some(
                        effect["message_id"]
                            .as_str()
                            .ok_or("missing message identity for deletion")?,
                    )
                };
                state.interaction.delete(platform, channel, message_id);
            }
            Some("panel") => {
                let panel = if effect["panel"].is_object() {
                    serde_json::from_value::<acheflow_extension_api::ExtensionPanel>(
                        effect["panel"].clone(),
                    )
                    .map_err(|_| "invalid extension panel")?
                } else {
                    acheflow_extension_api::ExtensionPanel {
                        extension_id: origin.into(),
                        title: self
                            .installed
                            .get(origin)
                            .map(|e| e.manifest.name.clone())
                            .unwrap_or_else(|| origin.into()),
                        widgets: vec![acheflow_extension_api::PanelWidget::Label {
                            text: effect["text"].as_str().unwrap_or_default().into(),
                        }],
                    }
                };
                if panel.extension_id != origin
                    || panel.title.len() > 200
                    || panel.widgets.len() > 64
                    || serde_json::to_vec(&panel)
                        .map_err(|_| "invalid panel")?
                        .len()
                        > 65536
                {
                    return Err("invalid extension panel size or identity".into());
                }
                state.panels.insert(origin.into(), panel);
            }
            Some("failure") => {
                return Err(effect["message"]
                    .as_str()
                    .unwrap_or("extension failed")
                    .chars()
                    .take(500)
                    .collect());
            }
            _ => return Err("unsupported host effect".into()),
        }
        let _=self.events.send(json!({"type":"event","origin":origin,"depth":depth,"event":effect,"now_ms":now_ms(),"request_id":format!("{}:{}",now_ms(),acheflow_media_api::workspace::new_token())}));
        Ok(())
    }
    /// Reserve each intent before any asynchronous side effect. Repeated ids never
    /// resend to a provider, including when the prior result is unknown.
    fn reserve(&self, request: &ControlRequest) -> Result<bool, String> {
        let now = now_ms();
        let at = request
            .request_id
            .split_once(':')
            .and_then(|(t, _)| t.parse::<u64>().ok())
            .ok_or("invalid_argument: request_id must be timestamp:unique_id")?;
        if request.request_id.len() > 160
            || at > now.saturating_add(10000)
            || now.saturating_sub(at) > 600000
        {
            return Err("conflict: expired request id".into());
        }
        let bytes = serde_json::to_vec(request).map_err(|_| "invalid_argument")?;
        if bytes.len() > 1024 * 1024 {
            return Err("invalid_argument: command exceeds 1 MiB".into());
        }
        let hash = openssl::sha::sha256(&bytes);
        let mut state = self.services.lock().unwrap();
        if let Some(old) = state.requests.iter().find(|r| r.id == request.request_id) {
            if old.hash != hash {
                return Err("conflict: request id reused with different arguments".into());
            }
            return old
                .result
                .clone()
                .ok_or("conflict: request is still in progress")?
                .map(|()| false);
        }
        if at <= state.expired_before {
            return Err("conflict: request result expired; submit a new intent".into());
        }
        if self
            .runtime
            .control_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != request.expected_revision
        {
            return Err("conflict: state revision changed; refresh before editing".into());
        }
        if state.requests.len() >= 1024 {
            let index = state
                .requests
                .iter()
                .position(|r| r.result.is_some())
                .ok_or("unavailable: command queue is full")?;
            let old = state.requests.remove(index).unwrap();
            state.expired_before = state.expired_before.max(old.at);
        }
        state.requests.push_back(RequestResult {
            id: request.request_id.clone(),
            hash,
            at,
            result: None,
        });
        Ok(true)
    }
    pub async fn execute(
        self: &Arc<Self>,
        request: ControlRequest,
    ) -> Result<ControlSnapshot, String> {
        let _command = self.command_lock.lock().await;
        if !self.owns(&request.workspace_id) {
            return Err("conflict: workspace is not active".into());
        }
        self.snapshot(0, 1);
        if !self.reserve(&request)? {
            return Ok(self.snapshot(0, 200));
        }
        let result = self.apply(&request).await;
        self.complete(&request, &result);
        result?;
        Ok(self.snapshot(0, 200))
    }
    fn complete(&self, request: &ControlRequest, result: &Result<(), String>) {
        let mut state = self.services.lock().unwrap();
        if state.owner.as_deref() == Some(&request.workspace_id) {
            if result.is_ok() && !matches!(request.action, ControlAction::SetScene { .. }) {
                self.runtime
                    .control_revision
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
            if let Some(entry) = state
                .requests
                .iter_mut()
                .find(|r| r.id == request.request_id)
            {
                entry.result = Some(result.clone());
            }
        }
    }
    pub fn apply_extension_scene(&self, request: ControlRequest) -> Result<(), String> {
        let _command = self
            .command_lock
            .try_lock()
            .map_err(|_| "conflict: application command in progress")?;
        if !self.owns(&request.workspace_id) {
            return Err("conflict: workspace changed".into());
        }
        self.snapshot(0, 1);
        if !self.reserve(&request)? {
            return Ok(());
        }
        let result = match self.runtime.control.try_lock() {
            Ok(_control) => match &request.action {
                ControlAction::SetScene { scene_id, spatial } => {
                    self.set_scene(&request.workspace_id, *scene_id, spatial)
                }
                _ => Err("forbidden: unsupported extension action".into()),
            },
            Err(_) => Err("conflict: media configuration in progress".into()),
        };
        self.complete(&request, &result);
        result
    }
    fn set_scene(
        &self,
        owner: &str,
        id: u32,
        spatial: &acheflow_media_api::spatial::SpatialScene,
    ) -> Result<(), String> {
        let mut workspace = self.runtime.workspace.write().unwrap();
        let workspace = workspace
            .as_mut()
            .filter(|w| w.id == owner)
            .ok_or("conflict: workspace changed")?;
        spatial.validate(&workspace.sources.iter().map(|s| s.id).collect::<Vec<_>>())?;
        let scene = workspace
            .scenes
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("not_found: scene")?;
        scene.spatial = Some(spatial.clone());
        workspace.edit_revision = self
            .runtime
            .control_revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        Ok(())
    }
    async fn apply(self: &Arc<Self>, request: &ControlRequest) -> Result<(), String> {
        match &request.action {
            ControlAction::ExtensionEvent {
                id,
                action_id,
                value,
            } => {
                if action_id.is_empty() || action_id.len() > 128 || value.len() > 8192 {
                    return Err("invalid_argument: extension action".into());
                }
                if !self
                    .services
                    .lock()
                    .unwrap()
                    .extensions
                    .iter()
                    .any(|e| e.id == *id && e.state == "running")
                {
                    return Err("unavailable: extension is not running".into());
                }
                let _=self.events.send(json!({"type":"action","origin":"ui","target":id,"action_id":action_id,"value":value,"depth":0,"now_ms":now_ms(),"request_id":request.request_id}));
            }
            ControlAction::SetScene { scene_id, spatial } => {
                let _control = self.runtime.control.lock().await;
                if self
                    .runtime
                    .control_revision
                    .load(std::sync::atomic::Ordering::Acquire)
                    != request.expected_revision
                {
                    return Err("conflict: state changed while waiting to edit the scene".into());
                }
                self.set_scene(&request.workspace_id, *scene_id, spatial)?;
            }
            ControlAction::CreatePoll { poll } => {
                let id = self
                    .services
                    .lock()
                    .unwrap()
                    .interaction
                    .create_poll(poll.clone(), now_ms())?;
                let options = poll
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{}: {s}", i + 1))
                    .collect::<Vec<_>>()
                    .join(" | ");
                let text = format!(
                    "Poll #{id}: {} — {options}. Vote: !vote <number>",
                    poll.question
                );
                for channel in &poll.channels {
                    if let Some(channel) = channel.strip_prefix("twitch:")
                        && let Err(error) = self.send_chat(channel, text.clone()).await
                    {
                        self.services
                            .lock()
                            .unwrap()
                            .interaction
                            .announcement_failed(id, error);
                    }
                }
            }
            ControlAction::ClosePoll { id, cancel } => self
                .services
                .lock()
                .unwrap()
                .interaction
                .close_poll(*id, *cancel)?,
            ControlAction::SetCommand { command } => self
                .services
                .lock()
                .unwrap()
                .interaction
                .set_command(command.clone())?,
            ControlAction::SendChat { channel_id, text } => {
                self.send_chat(channel_id, text.clone()).await?
            }
            ControlAction::SetExtension { id, enabled } => {
                crate::extension_host::set_enabled(
                    self.clone(),
                    &request.workspace_id,
                    id,
                    *enabled,
                )
                .await?
            }
            ControlAction::AuthorizeTwitch { client_id } => {
                crate::twitch_auth::begin(
                    self.clone(),
                    request.workspace_id.clone(),
                    client_id.clone(),
                )
                .await?
            }
        }
        Ok(())
    }
    async fn send_chat(&self, channel: &str, text: String) -> Result<(), String> {
        if text.trim().is_empty()
            || text.chars().count() > 500
            || !channel.bytes().all(|c| c.is_ascii_digit())
        {
            return Err("invalid_argument: chat channel or text".into());
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let outbound = crate::extension_host::Outbound {
            channel: channel.into(),
            text,
            result: tx,
        };
        {
            let running = self.twitch.lock().await;
            let running = running
                .as_ref()
                .ok_or("unavailable: Twitch extension is not running")?;
            running
                .sender
                .try_send(outbound)
                .map_err(|_| "unavailable: outgoing chat queue is full")?;
        }
        tokio::time::timeout(std::time::Duration::from_secs(15), rx)
            .await
            .map_err(|_| "outcome_unknown: chat send timed out")?
            .map_err(|_| "outcome_unknown: chat transport closed")?
    }
}

#[derive(Deserialize, Default)]
pub struct SnapshotQuery {
    pub after: Option<u64>,
    pub limit: Option<usize>,
}
pub async fn snapshot(
    State(host): State<Arc<ControlHost>>,
    Query(query): Query<SnapshotQuery>,
) -> Json<ControlSnapshot> {
    Json(host.snapshot(query.after.unwrap_or(0), query.limit.unwrap_or(200)))
}
pub async fn command(
    State(host): State<Arc<ControlHost>>,
    Json(request): Json<ControlRequest>,
) -> Result<Json<ControlSnapshot>, (StatusCode, Json<Value>)> {
    host.execute(request).await.map(Json).map_err(|message|{
        let status=if message.starts_with("conflict") {StatusCode::CONFLICT}else {StatusCode::BAD_REQUEST};
        (status,Json(json!({"error":{"code":message.split(':').next().unwrap_or("invalid_argument"),"message":message}})))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::{
        spatial::SpatialScene,
        workspace::{Orientation, Workspace},
    };
    pub fn host() -> (Arc<ControlHost>, Workspace) {
        let runtime = crate::live::Runtime::new(
            Arc::new(AsyncMutex::new(Default::default())),
            "127.0.0.1:1935".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_scene(Orientation::Horizontal);
        *runtime.workspace.write().unwrap() = Some(workspace.clone());
        let host = ControlHost::new(runtime);
        host.snapshot(0, 1);
        (host, workspace)
    }
    #[tokio::test]
    async fn loaded_guest_panel_actions_and_stop_complete_through_host_services() {
        let (host, workspace) = host();
        let request = |action, revision, suffix| ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:{suffix}", now_ms()),
            expected_revision: revision,
            action,
        };
        host.execute(request(
            ControlAction::SetExtension {
                id: "sample".into(),
                enabled: true,
            },
            0,
            "enable",
        ))
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if !host.snapshot(0, 1).panels.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        host.execute(request(
            ControlAction::ExtensionEvent {
                id: "sample".into(),
                action_id: "increment".into(),
                value: String::new(),
            },
            1,
            "action",
        ))
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let panels = host.snapshot(0, 1).panels;
                if serde_json::to_value(&panels)
                    .unwrap()
                    .to_string()
                    .contains("\"value\":\"1\"")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let state = host
            .execute(request(
                ControlAction::SetExtension {
                    id: "sample".into(),
                    enabled: false,
                },
                2,
                "stop",
            ))
            .await
            .unwrap();
        assert_eq!(
            state
                .extensions
                .iter()
                .find(|s| s.id == "sample")
                .unwrap()
                .state,
            "stopped"
        );
        assert!(host.guests.lock().await.is_empty());
    }
    #[tokio::test]
    async fn repeated_scene_intents_do_not_repeat_or_allow_changed_payloads() {
        let (host, workspace) = host();
        let mut request = ControlRequest {
            workspace_id: workspace.id,
            request_id: format!("{}:test", now_ms()),
            expected_revision: 0,
            action: ControlAction::SetScene {
                scene_id: 1,
                spatial: SpatialScene::from_canvas(&workspace.horizontal.base),
            },
        };
        assert_eq!(host.execute(request.clone()).await.unwrap().revision, 1);
        assert_eq!(host.execute(request.clone()).await.unwrap().revision, 1);
        request.expected_revision = 1;
        assert!(host.execute(request).await.unwrap_err().contains("reused"));
    }
    #[tokio::test]
    async fn stale_workspace_apply_cannot_erase_a_remote_scene_edit() {
        let (host, workspace) = host();
        let mut remote = SpatialScene::from_canvas(&workspace.horizontal.base);
        remote.objects[0].transform.rotation[1] = 45.;
        host.execute(ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:remote", now_ms()),
            expected_revision: 0,
            action: ControlAction::SetScene {
                scene_id: 1,
                spatial: remote,
            },
        })
        .await
        .unwrap();
        assert!(
            crate::live::configure(State(host.runtime.clone()), Json(workspace))
                .await
                .is_err()
        );
        let mut current = host.runtime.workspace.read().unwrap().clone().unwrap();
        current.scenes[0].spatial = None;
        assert!(
            crate::live::configure(State(host.runtime.clone()), Json(current))
                .await
                .is_ok()
        );
    }
    #[tokio::test]
    async fn wrong_workspace_stale_edits_and_poll_announcement_failure_are_explicit() {
        let (host, workspace) = host();
        let mut request = ControlRequest {
            workspace_id: "other".into(),
            request_id: format!("{}:test", now_ms()),
            expected_revision: 0,
            action: ControlAction::SetExtension {
                id: "sample".into(),
                enabled: true,
            },
        };
        assert!(host.execute(request.clone()).await.is_err());
        request.workspace_id = workspace.id;
        request.expected_revision = 10;
        assert!(host.execute(request.clone()).await.is_err());
        request.expected_revision = 0;
        request.action = ControlAction::CreatePoll {
            poll: acheflow_extension_api::interaction::PollRequest {
                question: "Test?".into(),
                options: vec!["A".into(), "B".into()],
                channels: vec!["twitch:123".into()],
                duration_seconds: 10,
            },
        };
        let result = host.execute(request).await.unwrap();
        assert_eq!(result.polls.len(), 1);
        assert!(result.polls[0].announcement_error.is_some());
    }
}
