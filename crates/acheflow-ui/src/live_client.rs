use crate::project::{Orientation, Workspace};
use acheflow_media_api::audio::{MAX_MIXER_EDITS, MixerAck, MixerEdit, MixerRequest};
use acheflow_media_api::live::{
    ApiError, ControlRequest, LiveStatus, ManageOutputsRequest, OutputAction, OutputProfile,
    OutputProfileAck, OutputProfileRequest, OutputTarget, SceneContent, SceneEdit,
    SceneSettingsAck, SceneSettingsRequest, SourceEdit, SourcePreviewRequest, SourceSettingsAck,
    SourceSettingsRequest, StartRequest, TakeSceneAck, TakeSceneRequest,
};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
pub(crate) struct LiveTelemetryInterval(pub Memo<Option<u32>>);

#[derive(Clone, Copy)]
pub struct LiveClient {
    pub project: Signal<Workspace>,
    pub status: Signal<Option<LiveStatus>>,
    pub busy: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub connection: Signal<Option<String>>,
    pub traffic: Signal<crate::traffic::History>,
    pub(crate) scoped_pending: Signal<bool>,
    scoped_edits: Signal<Vec<ScopedEdit>>,
    after_scoped_edits: Signal<Option<Action>>,
    generation: Signal<u64>,
}
enum ScopedEdit {
    Mixer(MixerEdit),
    Source(u32, SourceEdit),
    Scene(u32, SceneEdit),
    Output(OutputTarget, OutputProfile, u64),
}

#[derive(Clone, Copy)]
pub enum Action {
    Apply,
    Start {
        output: Option<OutputTarget>,
        preview_only: bool,
        scenes: [Option<u32>; 2],
    },
    Outputs(OutputAction, Option<OutputTarget>),
    Stop,
    Deactivate,
    DeactivateActive,
}

pub fn use_live_client(project: Signal<Workspace>) -> LiveClient {
    let telemetry = try_consume_context::<LiveTelemetryInterval>();
    let mut client = LiveClient {
        project,
        status: use_signal(|| None),
        busy: use_signal(|| false),
        error: use_signal(|| None),
        connection: use_signal(|| None),
        traffic: use_signal(crate::traffic::History::default),
        scoped_pending: use_signal(|| false),
        scoped_edits: use_signal(Vec::new),
        after_scoped_edits: use_signal(|| None),
        generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    let identity = use_memo(move || {
        let workspace = project.read();
        (
            workspace.id.clone(),
            workspace.server_url.clone(),
            crate::server_access::identity(),
        )
    });
    let mut previous_owner = use_signal(|| None);
    use_resource(move || {
        let owner = identity();
        let foreground = telemetry.and_then(|interval| (interval.0)());
        if previous_owner.peek().as_ref() != Some(&owner) {
            previous_owner.set(Some(owner.clone()));
            client.status.set(None);
            client.traffic.set(Default::default());
            client.error.set(None);
            client
                .connection
                .set(Some("Connecting to workspace server…".into()));
            client.busy.set(false);
            client.scoped_pending.set(false);
            client.scoped_edits.write().clear();
            client.after_scoped_edits.set(None);
            *client.generation.write() += 1;
        }
        async move {
            let mut failures = 0u8;
            loop {
                let started = web_time::Instant::now();
                if !*client.busy.peek() {
                    let generation = *client.generation.peek();
                    let workspace = project.peek().clone();
                    let response = request_media::<LiveStatus>(
                        &workspace,
                        reqwest::Method::GET,
                        "/api/live/status",
                        None,
                    )
                    .await;
                    if owner == identity()
                        && generation == *client.generation.peek()
                        && !*client.busy.peek()
                    {
                        match response {
                            Ok(status) => {
                                failures = 0;
                                if foreground.is_none() || client.traffic.peek().sample_due() {
                                    client.traffic.write().record(&status, &workspace);
                                }
                                if client.status.peek().as_ref() != Some(&status) {
                                    client.status.set(Some(status));
                                }
                                if client.connection.peek().is_some() {
                                    client.connection.set(None);
                                }
                            }
                            Err(error) => {
                                failures = failures.saturating_add(1);
                                client.status.set(None);
                                if client.connection.peek().as_ref() != Some(&error) {
                                    client.connection.set(Some(error));
                                }
                            }
                        }
                    }
                }
                let active = client.status.peek().as_ref().is_some_and(|s| {
                    s.workspace_id.as_deref() == Some(owner.0.as_str()) && !s.formats.is_empty()
                });
                let delay = live_polling_delay(foreground, active, failures);
                let elapsed = started.elapsed().as_millis().min(u128::from(delay)) as u32;
                crate::scene_preview::pause(delay - elapsed).await;
            }
        }
    });
    client
}

fn live_polling_delay(foreground_ms: Option<u32>, active: bool, failures: u8) -> u32 {
    match foreground_ms {
        Some(interval) => polling_delay(true, failures, interval),
        None => polling_delay(active, failures, 500),
    }
}

impl LiveClient {
    pub(crate) fn save_output_profile(self, target: OutputTarget) {
        let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>() else {
            return;
        };
        if *self.busy.peek() || *self.scoped_pending.peek() {
            return;
        }
        if let Some(profile) = sync.changed_output_profile(target) {
            let revision = self.project.peek().edit_revision;
            self.queue_edit(sync, ScopedEdit::Output(target, profile, revision));
        }
    }

    pub(crate) fn save_scene(self, id: u32) {
        if *self.busy.peek() || *self.scoped_pending.peek() || !self.owns() {
            return;
        }
        let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>() else {
            return;
        };
        let Some(state) = sync
            .scene_save_state(id)
            .filter(|state| state.available && state.changed)
        else {
            return;
        };
        let Some(scene) = self
            .project
            .peek()
            .scenes
            .iter()
            .find(|scene| scene.id == id)
            .cloned()
        else {
            return;
        };
        let edit = if state.saved {
            SceneEdit::Content(SceneContent::of(&scene))
        } else {
            SceneEdit::Register(scene)
        };
        self.queue_edit(sync, ScopedEdit::Scene(id, edit));
    }

    pub(crate) fn remove_scene(mut self, id: u32) {
        if *self.busy.peek() || *self.scoped_pending.peek() {
            return;
        }
        if let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>()
            && self.owns()
            && sync.scene_save_state(id).is_some_and(|state| state.saved)
        {
            self.queue_edit(sync, ScopedEdit::Scene(id, SceneEdit::Remove));
        } else {
            let _ = SceneEdit::Remove.apply(&mut self.project.write(), id);
        }
    }

    pub(crate) fn take_scene(mut self, orientation: Orientation, scene_id: u32) {
        if *self.busy.peek() || *self.scoped_pending.peek() {
            return;
        }
        let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>()
            .filter(|sync| sync.can_take_scene(orientation, scene_id))
        else {
            return;
        };
        let target = self.project.peek().clone();
        let request = TakeSceneRequest {
            workspace_id: target.id.clone(),
            expected_revision: target.edit_revision,
            orientation,
            scene_id,
        };
        let identity = crate::server_access::identity();
        self.busy.set(true);
        self.error.set(None);
        *self.generation.write() += 1;
        let generation = *self.generation.peek();
        spawn(async move {
            let result: Result<TakeSceneAck, String> = match serde_json::to_value(&request) {
                Ok(body) => {
                    request_media(&target, reqwest::Method::POST, "/api/live/take", Some(body))
                        .await
                }
                Err(error) => Err(error.to_string()),
            };
            if generation != *self.generation.peek()
                || self.project.peek().id != target.id
                || self.project.peek().server_url != target.server_url
                || crate::server_access::identity() != identity
            {
                return;
            }
            match result {
                Ok(ack) if sync.applied_program(&request, ack) => {}
                Ok(_) => self.error.set(Some(
                    "Program scene changed on the server. Refreshing workspace state.".into(),
                )),
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }

    pub(crate) fn mixer_edit(mut self, edit: MixerEdit) {
        let changed = match edit.apply(&mut self.project.write()) {
            Ok(changed) => changed,
            Err(error) => {
                self.error.set(Some(error));
                return;
            }
        };
        if !changed || !self.owns() {
            return;
        }
        let Some(sync) = try_consume_context::<crate::workspace_sync::WorkspaceSync>()
            .filter(|sync| sync.mixer_has_target(&edit))
        else {
            return;
        };
        self.queue_edit(sync, ScopedEdit::Mixer(edit));
    }

    pub(crate) fn source_edit(mut self, source_id: u32, edit: SourceEdit) {
        let sync =
            try_consume_context::<crate::workspace_sync::WorkspaceSync>().filter(|_| self.owns());
        let saved = sync.and_then(|sync| sync.saved_source(source_id));
        let local_removal = matches!(edit, SourceEdit::Remove) && saved.is_none();
        if local_removal && *self.scoped_pending.peek() {
            return;
        }
        if matches!(
            edit,
            SourceEdit::Name(_) | SourceEdit::PreviewWidth(_) | SourceEdit::PreviewHeight(_)
        ) || local_removal
        {
            let changed = edit.apply(&mut self.project.write(), source_id);
            match changed {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    self.error.set(Some(error));
                    return;
                }
            }
        }
        let Some(sync) = sync else { return };
        if matches!(edit, SourceEdit::Register(_)) {
            if saved.is_some() || !sync.can_edit_workspace() {
                return;
            }
        } else if saved.is_none() {
            return;
        }
        self.queue_edit(sync, ScopedEdit::Source(source_id, edit));
    }

    fn queue_edit(mut self, sync: crate::workspace_sync::WorkspaceSync, edit: ScopedEdit) {
        {
            let mut edits = self.scoped_edits.write();
            match (edits.last_mut(), edit) {
                (Some(ScopedEdit::Mixer(previous)), ScopedEdit::Mixer(edit))
                    if edit.replaces(previous) =>
                {
                    *previous = edit
                }
                (_, edit) => edits.push(edit),
            }
        }
        if *self.scoped_pending.peek() {
            return;
        }
        self.scoped_pending.set(true);
        self.error.set(None);
        let identity = crate::server_access::identity();
        let owner = self.project.peek().id.clone();
        let endpoint = self.project.peek().server_url.clone();
        let same_workspace = move || {
            let workspace = self.project.peek();
            workspace.id == owner
                && workspace.server_url == endpoint
                && crate::server_access::identity() == identity
        };
        spawn(async move {
            loop {
                crate::scene_preview::pause(75).await;
                if !same_workspace() {
                    return;
                }
                if self.scoped_edits.peek().is_empty() {
                    break;
                }
                if *self.busy.peek() {
                    continue;
                }
                let workspace = self.project.peek().clone();
                let edits = {
                    let mut queue = self.scoped_edits.write();
                    let count = if matches!(queue.first(), Some(ScopedEdit::Mixer(_))) {
                        queue
                            .iter()
                            .take_while(|edit| matches!(edit, ScopedEdit::Mixer(_)))
                            .take(MAX_MIXER_EDITS)
                            .count()
                    } else {
                        1
                    };
                    queue.drain(..count).collect::<Vec<_>>()
                };
                let error = match &edits[..] {
                    [ScopedEdit::Output(target, profile, revision)] => {
                        let request = OutputProfileRequest {
                            workspace_id: workspace.id.clone(),
                            expected_revision: *revision,
                            target: *target,
                            profile: profile.clone(),
                        };
                        let result = match serde_json::to_value(&request) {
                            Ok(body) => {
                                request_media::<OutputProfileAck>(
                                    &workspace,
                                    reqwest::Method::POST,
                                    "/api/live/output/profile",
                                    Some(body),
                                )
                                .await
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        if !same_workspace() {
                            return;
                        }
                        match result {
                            Ok(ack) if sync.applied_output_profile(&request, ack) => continue,
                            Ok(_) => {
                                "Workspace version changed while saving the output profile.".into()
                            }
                            Err(error) => error,
                        }
                    }
                    [ScopedEdit::Scene(id, edit)] => {
                        let request = SceneSettingsRequest {
                            workspace_id: workspace.id.clone(),
                            expected_revision: workspace.edit_revision,
                            scene_id: *id,
                            edit: edit.clone(),
                        };
                        let result = send_scene(&workspace, &request).await;
                        if !same_workspace() {
                            return;
                        }
                        match result {
                            Ok(ack) if sync.applied_scene(&request, ack) => continue,
                            Ok(_) => "Workspace version changed while saving the scene.".into(),
                            Err(error) => error,
                        }
                    }
                    [ScopedEdit::Source(id, edit)] => {
                        let request = SourceSettingsRequest {
                            workspace_id: workspace.id.clone(),
                            expected_revision: workspace.edit_revision,
                            source_id: *id,
                            edit: edit.clone(),
                        };
                        let result = send_source(&workspace, &request).await;
                        if !same_workspace() {
                            return;
                        }
                        match result {
                            Ok(ack) if sync.applied_source(&request, ack) => continue,
                            Ok(_) => {
                                "Workspace version changed while saving source settings.".into()
                            }
                            Err(error) => error,
                        }
                    }
                    _ => {
                        let request = MixerRequest {
                            workspace_id: workspace.id.clone(),
                            expected_revision: workspace.edit_revision,
                            edits: edits
                                .into_iter()
                                .filter_map(|edit| match edit {
                                    ScopedEdit::Mixer(edit) => Some(edit),
                                    _ => None,
                                })
                                .collect(),
                        };
                        let result = send_mixer(&workspace, &request).await;
                        if !same_workspace() {
                            return;
                        }
                        match result {
                            Ok(ack) if sync.applied_mixer(&request, ack) => continue,
                            Ok(_) => "Workspace version changed while saving the mixer.".into(),
                            Err(error) => error,
                        }
                    }
                };
                self.error
                    .set(Some(format!("{error} Changes were kept in your draft.")));
                self.scoped_edits.write().clear();
                break;
            }
            self.scoped_pending.set(false);
            let action = self.after_scoped_edits.write().take();
            if let Some(action) = action {
                self.command(action);
            }
        });
    }

    pub fn owns(&self) -> bool {
        self.status
            .read()
            .as_ref()
            .is_some_and(|s| s.workspace_id.as_deref() == Some(self.project.read().id.as_str()))
    }
    pub fn running(&self) -> bool {
        self.owns()
            && self
                .status
                .read()
                .as_ref()
                .is_some_and(|s| !s.formats.is_empty())
    }
    pub fn command(mut self, action: Action) {
        if (self.busy)() {
            return;
        }
        if *self.scoped_pending.peek() && matches!(action, Action::Apply | Action::Start { .. }) {
            self.after_scoped_edits.set(Some(action));
            return;
        }
        crate::diagnostics::record(
            "info",
            "media",
            match action {
                Action::Apply => "Applying workspace",
                Action::Start {
                    preview_only: true, ..
                } => "Starting preview",
                Action::Start { .. } | Action::Outputs(OutputAction::Start, _) => {
                    "Starting outputs"
                }
                Action::Outputs(OutputAction::Restart, _) => "Restarting outputs",
                Action::Stop => "Stopping stream",
                Action::Deactivate | Action::DeactivateActive => "Deactivating workspace",
            },
        );
        let mut workspace = self.project.peek().clone();
        if let Action::Start { scenes, .. } = action {
            for (orientation, scene) in Orientation::ALL.into_iter().zip(scenes) {
                workspace.set_active(orientation, scene);
            }
        }
        let applies_workspace = matches!(&action, Action::Apply | Action::Start { .. });
        let target = workspace.clone();
        let identity = crate::server_access::identity();
        let sync = try_consume_context::<crate::workspace_sync::WorkspaceSync>();
        self.busy.set(true);
        self.error.set(None);
        *self.generation.write() += 1;
        let generation = *self.generation.peek();
        spawn(async move {
            let (method, path, body) = match action {
                Action::Apply => (
                    reqwest::Method::PUT,
                    "/api/live/config",
                    serde_json::to_value(workspace.server_configuration()),
                ),
                Action::Start {
                    output,
                    preview_only,
                    ..
                } => {
                    let formats = Orientation::ALL
                        .into_iter()
                        .filter(|o| workspace.active(*o).is_some())
                        .collect();
                    (
                        reqwest::Method::POST,
                        "/api/live/start",
                        serde_json::to_value(StartRequest {
                            workspace: workspace.server_configuration(),
                            formats,
                            preview_only,
                            output,
                        }),
                    )
                }
                Action::Outputs(action, output) => (
                    reqwest::Method::POST,
                    "/api/live/outputs",
                    serde_json::to_value(ManageOutputsRequest {
                        workspace_id: workspace.id,
                        action,
                        output,
                    }),
                ),
                Action::Stop => (
                    reqwest::Method::POST,
                    "/api/live/stop",
                    serde_json::to_value(ControlRequest {
                        workspace_id: workspace.id,
                    }),
                ),
                Action::Deactivate => (
                    reqwest::Method::DELETE,
                    "/api/live/config",
                    serde_json::to_value(ControlRequest {
                        workspace_id: workspace.id,
                    }),
                ),
                Action::DeactivateActive => (
                    reqwest::Method::DELETE,
                    "/api/live/config",
                    serde_json::to_value(ControlRequest {
                        workspace_id: self
                            .status
                            .peek()
                            .as_ref()
                            .and_then(|s| s.workspace_id.clone())
                            .unwrap_or_default(),
                    }),
                ),
            };
            let result: Result<LiveStatus, String> = match body {
                Ok(body) => request_media(&target, method, path, Some(body)).await,
                Err(e) => Err(e.to_string()),
            };
            if generation != *self.generation.peek()
                || self.project.peek().id != target.id
                || self.project.peek().server_url != target.server_url
                || crate::server_access::identity() != identity
            {
                return;
            }
            match result {
                Ok(status) => {
                    if applies_workspace
                        && status.workspace_id.as_deref() == Some(target.id.as_str())
                    {
                        if let Action::Start { scenes, .. } = action {
                            let mut project = self.project.write();
                            for (orientation, scene) in Orientation::ALL.into_iter().zip(scenes) {
                                project.set_active(orientation, scene);
                            }
                        }
                        if let Some(sync) = sync {
                            sync.applied(&target, status.edit_revision);
                        } else {
                            self.project.write().edit_revision = status.edit_revision;
                        }
                    }
                    self.status.set(Some(status));
                    self.connection.set(None);
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn add_source(mut self) -> Option<u32> {
        let url = self
            .status
            .peek()
            .as_ref()
            .ok_or_else(|| "Connect to the workspace server before adding a source.".to_owned())
            .and_then(|status| ingest_url(&self.project.peek(), status));
        let url = match url {
            Ok(url) => url,
            Err(error) => {
                self.error.set(Some(error));
                return None;
            }
        };
        let mut project = self.project.write();
        project.add_source();
        let source = project.sources.last_mut()?;
        source.url = url;
        Some(source.id)
    }
    pub async fn copy_text(mut self, text: String) -> bool {
        #[cfg(target_arch = "wasm32")]
        let result = match web_sys::window() {
            Some(window) => wasm_bindgen_futures::JsFuture::from(
                window.navigator().clipboard().write_text(&text),
            )
            .await
            .map(|_| ())
            .map_err(|_| {
                "Clipboard request was denied; reveal the token to copy it manually".to_owned()
            }),
            None => Err("Browser clipboard is unavailable".into()),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result =
            try_consume_context::<std::sync::Arc<dyn blitz_traits::shell::ShellProvider>>()
                .ok_or_else(|| "Clipboard is unavailable".to_owned())
                .and_then(|shell| {
                    shell
                        .set_clipboard_text(text)
                        .map_err(|_| "Clipboard request failed".to_owned())
                });
        match result {
            Ok(()) => true,
            Err(error) => {
                self.error.set(Some(error));
                false
            }
        }
    }
    pub fn preview(&self, path: &str) -> String {
        crate::media::server_url(
            &self.project.read(),
            &format!("{path}?workspace={}", self.project.read().id),
        )
        .unwrap_or_default()
    }

    pub fn srt_url(&self, port: u16) -> Result<String, String> {
        let server = crate::media::server_url(&self.project.read(), "")?;
        let server = reqwest::Url::parse(&server).map_err(|error| error.to_string())?;
        let host = server.host_str().ok_or("The server address has no host.")?;
        Ok(format!("srt://{host}:{port}?mode=caller&transtype=live"))
    }
}

pub(crate) async fn renew_source_preview(
    workspace: &Workspace,
    source_id: u32,
) -> Result<(), String> {
    let body = serde_json::to_value(SourcePreviewRequest {
        workspace_id: workspace.id.clone(),
        source_id,
    })
    .map_err(|error| error.to_string())?;
    request_media(
        workspace,
        reqwest::Method::POST,
        "/api/live/source-preview",
        Some(body),
    )
    .await
}

async fn send_scene(
    workspace: &Workspace,
    request: &SceneSettingsRequest,
) -> Result<SceneSettingsAck, String> {
    let body = serde_json::to_value(request).map_err(|error| error.to_string())?;
    request_media(
        workspace,
        reqwest::Method::POST,
        "/api/live/scenes",
        Some(body),
    )
    .await
}

async fn send_source(
    workspace: &Workspace,
    request: &SourceSettingsRequest,
) -> Result<SourceSettingsAck, String> {
    let body = serde_json::to_value(request).map_err(|error| error.to_string())?;
    request_media(
        workspace,
        reqwest::Method::POST,
        "/api/live/sources",
        Some(body),
    )
    .await
}

async fn send_mixer(workspace: &Workspace, request: &MixerRequest) -> Result<MixerAck, String> {
    let body = serde_json::to_value(request).map_err(|error| error.to_string())?;
    request_media(
        workspace,
        reqwest::Method::POST,
        "/api/live/mixer",
        Some(body),
    )
    .await
}

fn ingest_url(workspace: &Workspace, status: &LiveStatus) -> Result<String, String> {
    let port = status
        .rtmp_address
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "The server returned an invalid RTMP listener address.".to_owned())?
        .port();
    let base = reqwest::Url::parse(&crate::media::server_url(workspace, "")?)
        .map_err(|e| e.to_string())?;
    let host = base.host_str().ok_or("The server address has no host.")?;
    let scheme = if status.rtmp_tls { "rtmps" } else { "rtmp" };
    Ok(format!("{scheme}://{host}:{port}/live"))
}

async fn request_media<T: serde::de::DeserializeOwned>(
    workspace: &Workspace,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T, String> {
    let mut request =
        crate::server_access::request(method, &crate::media::server_url(workspace, path)?)?
            .timeout(std::time::Duration::from_secs(15));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Media server is unreachable: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        if let Ok(error) = serde_json::from_slice::<ApiError>(&bytes) {
            return Err(error.error);
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(message) = value["error"]["message"].as_str()
        {
            return Err(message.to_owned());
        }
        return Err(format!("Media API rejected the request ({status})"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Invalid media response: {e}"))
}
pub(crate) fn polling_delay(active: bool, failures: u8, active_ms: u32) -> u32 {
    if failures > 0 {
        (1000u32 << failures.min(5)).min(30_000)
    } else if active {
        active_ms
    } else {
        5000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_is_slow_when_idle_and_backs_off_when_unreachable() {
        assert_eq!(live_polling_delay(Some(100), false, 0), 100);
        assert_eq!(live_polling_delay(Some(250), false, 0), 250);
        assert_eq!(live_polling_delay(None, false, 0), 5000);
        assert_eq!(live_polling_delay(Some(100), true, 1), 2000);
        assert_eq!(polling_delay(true, 0, 500), 500);
        assert_eq!(polling_delay(false, 0, 500), 5000);
        assert_eq!(polling_delay(true, 0, 1000), 1000);
        assert!(polling_delay(false, 2, 500) > polling_delay(false, 1, 500));
        assert_eq!(polling_delay(true, 200, 1000), 30_000);
    }

    #[test]
    fn ingest_uses_selected_server_host_and_discovered_listener_port_and_tls() {
        let mut workspace = Workspace {
            server_url: "https://remote.example:8443/proxy".into(),
            ..Workspace::default()
        };
        let mut status = LiveStatus {
            edit_revision: 0,
            rtmp_address: "0.0.0.0:1940".into(),
            rtmp_tls: true,
            ..LiveStatus::default()
        };
        assert_eq!(
            ingest_url(&workspace, &status).unwrap(),
            "rtmps://remote.example:1940/live"
        );
        workspace.server_url = "http://[::1]:21296".into();
        status.rtmp_address = "[::]:1936".into();
        status.rtmp_tls = false;
        assert_eq!(
            ingest_url(&workspace, &status).unwrap(),
            "rtmp://[::1]:1936/live"
        );
        status.rtmp_address = "invalid".into();
        assert!(ingest_url(&workspace, &status).is_err());
    }
}

#[component]
pub fn LiveNotice() -> Element {
    let client = use_context::<LiveClient>();
    rsx! {
        if let Some(error)=client.connection.read().as_ref() {p {class:"live-notice",role:"status","{error}"}}
        if let Some(error)=client.error.read().as_ref() {p {class:"live-notice field-error",role:"alert","{error}"}}
        if !client.owns() && client.status.read().as_ref().is_some_and(|s|s.workspace_id.is_some()) {
            div {class:"live-notice row",span {"The server is configured for another workspace."}button {disabled:(client.busy)(),onclick:move |_|client.command(Action::DeactivateActive),"Stop & deactivate active workspace"}}
        }
    }
}
