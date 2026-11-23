use crate::{
    compositor::RunningFormat,
    ingest::{Input, Publisher},
};
use acheflow_media_api::{
    MediaCapabilities,
    audio::{MAX_MIXER_EDITS, MixerAck, MixerRequest},
    live::{
        ApiError, ControlRequest, LiveStatus, ManageOutputsRequest, OutputAction, OutputProfileAck,
        OutputProfileRequest, RestartOutputRequest, SceneEdit, SceneSettingsAck,
        SceneSettingsRequest, SourceCapture, SourcePreviewRequest, SourceSettingsAck,
        SourceSettingsRequest, StartRequest, TakeSceneAck, TakeSceneRequest,
    },
    workspace::{Output, Resolution, Workspace},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock, Weak, atomic::Ordering},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

pub type RuntimeState = Arc<Runtime>;
pub type ApiResult = Result<Json<LiveStatus>, (StatusCode, Json<ApiError>)>;
type InputWorker = tokio::task::JoinHandle<()>;
const SOURCE_PREVIEW_IDLE: std::time::Duration = std::time::Duration::from_secs(7);

struct InputChanges {
    retired: Vec<InputWorker>,
    managed: Vec<Arc<Input>>,
}

pub struct Runtime {
    pub visual_services: RwLock<Option<Arc<Mutex<crate::control::Services>>>>,
    pub control_revision: std::sync::atomic::AtomicU64,
    pub workspace: RwLock<Option<Workspace>>,
    pub inputs: Mutex<HashMap<u32, Arc<Input>>>,
    source_previews: Mutex<HashMap<u32, tokio::time::Instant>>,
    pub formats: AsyncMutex<Vec<RunningFormat>>,
    pub control: AsyncMutex<()>,
    pub capabilities: crate::ffmpeg::CapabilityState,
    pub rtmp_address: String,
    pub rtmp_tls: std::sync::atomic::AtomicBool,
    pub shutdown: CancellationToken,
    pub activated: std::sync::atomic::AtomicBool,
    monitor: crate::media_monitor::ServerMonitor,
    saved: Mutex<(u64, Option<crate::workspace_store::Store>)>,
}

impl Runtime {
    pub fn new(capabilities: crate::ffmpeg::CapabilityState, rtmp_address: String) -> Arc<Self> {
        Arc::new(Self {
            visual_services: RwLock::new(None),
            control_revision: std::sync::atomic::AtomicU64::new(0),
            workspace: RwLock::new(None),
            inputs: Mutex::new(HashMap::new()),
            source_previews: Mutex::new(HashMap::new()),
            formats: AsyncMutex::new(vec![]),
            control: AsyncMutex::new(()),
            capabilities,
            rtmp_address,
            rtmp_tls: std::sync::atomic::AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            activated: std::sync::atomic::AtomicBool::new(false),
            monitor: Default::default(),
            saved: Mutex::new((0, None)),
        })
    }

    pub async fn status(&self) -> LiveStatus {
        let running = self.formats.lock().await;
        let workspace = self.workspace.read().unwrap();
        let workspace = workspace.as_ref();
        let workspace_id = workspace.map(|workspace| workspace.id.clone());
        let sources = self
            .inputs
            .lock()
            .unwrap()
            .values()
            .map(|i| i.status())
            .collect();
        let formats = running
            .iter()
            .map(|format| {
                let scene = workspace.and_then(|workspace| {
                    let id = workspace.active(format.orientation)?;
                    workspace.scenes.iter().find(|scene| scene.id == id)
                });
                let outputs = workspace
                    .map(|w| w.stream(format.orientation).outputs.as_slice())
                    .unwrap_or_default();
                format.status(scene, outputs)
            })
            .collect();
        LiveStatus {
            activated: self.activated.load(Ordering::Acquire),
            edit_revision: workspace.map_or(0, |workspace| workspace.edit_revision),
            workspace_id,
            rtmp_address: self.rtmp_address.clone(),
            rtmp_tls: self.rtmp_tls.load(Ordering::Relaxed),
            sources,
            formats,
            server_load: self.monitor.samples(),
        }
    }

    pub fn attach_store(&self, store: crate::workspace_store::Store) -> Result<(), String> {
        let (revision, workspace) = store.load()?;
        let mut current = self.workspace.write().unwrap();
        *current = workspace;
        *self.saved.lock().unwrap() = (revision, Some(store));
        let mut random = [0; 8];
        openssl::rand::rand_bytes(&mut random)
            .map_err(|_| "Cannot establish server control generation.")?;
        self.control_revision.store(
            u64::from_le_bytes(random) & ((1u64 << 52) - 1),
            Ordering::Release,
        );
        self.activated.store(false, Ordering::Release);
        Ok(())
    }
    fn revision_for(&self, current: &Option<Workspace>) -> u64 {
        self.saved
            .lock()
            .unwrap()
            .0
            .max(current.as_ref().map(|w| w.edit_revision).unwrap_or(0))
    }
    pub fn saved_state(&self) -> (u64, Option<Workspace>) {
        let current = self.workspace.read().unwrap();
        (self.revision_for(&current), current.clone())
    }
    fn commit(
        &self,
        current: &mut Option<Workspace>,
        mut next: Option<Workspace>,
    ) -> Result<(), String> {
        let mut saved = self.saved.lock().unwrap();
        let revision = saved
            .0
            .max(current.as_ref().map(|w| w.edit_revision).unwrap_or(0))
            .checked_add(1)
            .ok_or("Workspace revision exhausted.")?;
        if let Some(workspace) = &mut next {
            workspace.edit_revision = revision;
            workspace.server_url.clear();
        }
        if let Some(store) = &saved.1 {
            store.save(revision, next.as_ref())?;
        }
        saved.0 = revision;
        *current = next;
        self.control_revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    pub fn update_workspace(
        &self,
        owner: &str,
        update: impl FnOnce(&mut Workspace) -> Result<(), String>,
    ) -> Result<(), String> {
        self.update_workspace_checked(owner, None, update)
            .map(|_| ())
    }

    fn update_workspace_checked(
        &self,
        owner: &str,
        expected: Option<u64>,
        update: impl FnOnce(&mut Workspace) -> Result<(), String>,
    ) -> Result<u64, String> {
        let mut current = self.workspace.write().unwrap();
        if expected.is_some_and(|revision| revision != self.revision_for(&current)) {
            return Err(
                "conflict: Workspace changed on the server. Refresh before retrying.".into(),
            );
        }
        let mut next = current
            .as_ref()
            .filter(|w| w.id == owner)
            .cloned()
            .ok_or("conflict: workspace changed")?;
        update(&mut next)?;
        validate(&next)?;
        self.commit(&mut current, Some(next))?;
        Ok(self.revision_for(&current))
    }
    pub(crate) async fn install(
        &self,
        mut workspace: Workspace,
        activate: bool,
    ) -> Result<(), String> {
        workspace.migrate_spatial();
        validate(&workspace)?;
        let srt_available = self.capabilities.lock().await.srt_input;
        let formats = self.formats.lock().await;
        let InputChanges { retired, managed } =
            self.install_inputs(workspace, activate, &formats)?;
        for worker in retired {
            let _ = worker.await;
        }
        for input in managed {
            self.start_input(&input, srt_available).await;
        }
        Ok(())
    }

    async fn start_input(&self, input: &Arc<Input>, srt_available: bool) {
        use acheflow_media_api::visual::SourceKind;
        match &input.config.kind {
            SourceKind::Srt { port } => match self.rtmp_address.parse::<std::net::SocketAddr>() {
                Ok(mut address) if srt_available => {
                    address.set_port(*port);
                    crate::srt_input::start(input, address, &self.shutdown);
                }
                _ => {
                    let worker = input.stop_managed();
                    if let Some(worker) = worker {
                        let _ = worker.await;
                    }
                    *input.error.lock().unwrap() =
                        Some("SRT listener is unavailable on this server".into());
                }
            },
            SourceKind::Camera { .. } => {
                crate::camera_input::start(input, &self.shutdown);
            }
            SourceKind::Microphone { .. } => {
                crate::microphone_input::start(input, &self.shutdown);
            }
            _ => {}
        }
    }

    async fn remove_input(&self, id: u32) {
        let input = self.inputs.lock().unwrap().remove(&id);
        if let Some(input) = input {
            input.disconnect();
            if let Some(worker) = input.stop_managed() {
                let _ = worker.await;
            }
        }
    }

    async fn renew_source_preview(
        self: &Arc<Self>,
        request: SourcePreviewRequest,
    ) -> Result<(), String> {
        let _control = self.control.lock().await;
        if self.shutdown.is_cancelled() {
            return Err("Media server is shutting down".into());
        }
        let id = request.source_id;
        let source = {
            let workspace = self.workspace.read().unwrap();
            let workspace = workspace
                .as_ref()
                .filter(|workspace| workspace.id == request.workspace_id)
                .ok_or(
                    "conflict: Workspace must be saved on this server before previewing a source",
                )?;
            workspace
                .sources
                .iter()
                .find(|source| source.id == id && source.kind.is_media_input())
                .cloned()
                .ok_or("Save the media source before previewing it")?
        };
        if self.activated.load(Ordering::Acquire) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + SOURCE_PREVIEW_IDLE;
        {
            let mut previews = self.source_previews.lock().unwrap();
            if let Some(current) = previews.get_mut(&id) {
                *current = deadline;
                return Ok(());
            }
        }
        let srt_available = self.capabilities.lock().await.srt_input;
        let input = Arc::new(Input::new(source));
        self.source_previews.lock().unwrap().insert(id, deadline);
        self.inputs.lock().unwrap().insert(id, input.clone());
        tokio::spawn(self.clone().expire_source_preview(input.clone()));
        self.start_input(&input, srt_available).await;
        Ok(())
    }

    async fn expire_source_preview(self: Arc<Self>, input: Arc<Input>) {
        let id = input.config.id;
        loop {
            let deadline = self.source_previews.lock().unwrap().get(&id).copied();
            let Some(deadline) = deadline else {
                return;
            };
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                _ = tokio::time::sleep_until(deadline) => {}
            }
            let _control = self.control.lock().await;
            if self
                .inputs
                .lock()
                .unwrap()
                .get(&id)
                .is_none_or(|current| !Arc::ptr_eq(current, &input))
            {
                return;
            }
            let expired = {
                let mut previews = self.source_previews.lock().unwrap();
                match previews.get(&id) {
                    None => return,
                    Some(deadline) if *deadline <= tokio::time::Instant::now() => {
                        previews.remove(&id);
                        true
                    }
                    Some(_) => false,
                }
            };
            if expired {
                self.remove_input(id).await;
                return;
            }
        }
    }

    async fn refresh_source_capture(&self, id: u32) {
        if !self.activated.load(Ordering::Acquire) {
            if self.source_previews.lock().unwrap().remove(&id).is_some() {
                self.remove_input(id).await;
            }
            return;
        }
        let source = self
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .and_then(|workspace| {
                workspace
                    .sources
                    .iter()
                    .find(|source| source.id == id && source.kind.is_media_input())
                    .cloned()
            });
        let Some(source) = source else {
            self.remove_input(id).await;
            return;
        };
        let (input, retired) = {
            let mut inputs = self.inputs.lock().unwrap();
            let capture = SourceCapture::of(&source);
            if inputs
                .get(&id)
                .is_some_and(|input| capture.matches(&input.config))
            {
                return;
            }
            let input = Arc::new(Input::new(source));
            let retired = inputs.insert(id, input.clone()).and_then(|old| {
                old.disconnect();
                old.stop_managed()
            });
            (input, retired)
        };
        if let Some(worker) = retired {
            let _ = worker.await;
        }
        let srt_available = self.capabilities.lock().await.srt_input;
        self.start_input(&input, srt_available).await;
    }

    fn install_inputs(
        &self,
        workspace: Workspace,
        activate: bool,
        formats: &[RunningFormat],
    ) -> Result<InputChanges, String> {
        let mut current = self.workspace.write().unwrap();
        if workspace.edit_revision != self.revision_for(&current) {
            return Err("conflict: Workspace changed on the server. Refresh or explicitly resolve your local edits before applying.".into());
        }
        if !formats.is_empty() {
            let old = current
                .as_ref()
                .ok_or("conflict: another workspace is running")?;
            if !activate || old.id != workspace.id {
                return Err(
                    "conflict: Stop streaming before replacing the server workspace.".into(),
                );
            }
            for format in formats {
                if !old.same_output_profile(&workspace, format.orientation) {
                    return Err("Stop streaming before changing resolution, FPS or outputs. Scene layers, mixers and audio routes can be changed live.".into());
                }
            }
        }
        let mut inputs = self.inputs.lock().unwrap();
        let switching = current.as_ref().is_some_and(|old| old.id != workspace.id);
        if (!activate || switching)
            && self.activated.load(Ordering::Acquire)
            && inputs
                .values()
                .any(|input| input.connected.load(Ordering::Acquire))
        {
            return Err(
                "conflict: Deactivate connected sources before replacing the server workspace."
                    .into(),
            );
        }
        // Publish neither the new configuration nor input cancellations until saving succeeds.
        self.commit(&mut current, Some(workspace))?;
        // Matching inputs are promoted in place. Their preview expiry tasks
        // no longer own them after activation or workspace replacement.
        self.source_previews.lock().unwrap().clear();
        let workspace = current.as_ref().unwrap();
        let mut retired = Vec::new();
        inputs.retain(|id, input| {
            let keep = activate
                && !switching
                && workspace.sources.iter().any(|s| {
                    s.kind.is_media_input()
                        && s.id == *id
                        && s.kind == input.config.kind
                        && s.key == input.config.key
                        && s.url == input.config.url
                });
            if !keep {
                input.disconnect();
                retired.extend(input.stop_managed());
            }
            keep
        });
        if activate {
            for source in workspace.sources.iter().filter(|s| s.kind.is_media_input()) {
                inputs
                    .entry(source.id)
                    .or_insert_with(|| Arc::new(Input::new(source.clone())));
            }
        }
        self.activated.store(activate, Ordering::Release);
        let managed = inputs
            .values()
            .filter(|i| !i.config.kind.is_rtmp())
            .cloned()
            .collect();
        Ok(InputChanges { retired, managed })
    }
    async fn apply(&self, workspace: Workspace) -> Result<(), String> {
        self.install(workspace, true).await
    }

    pub fn authorize(
        &self,
        app: &str,
        key: &str,
        parent: &CancellationToken,
    ) -> Result<Publisher, String> {
        let inputs = self.inputs.lock().unwrap();
        let input = inputs
            .values()
            .find(|input| {
                input.config.kind.is_rtmp()
                    && bool::from(input.config.key.as_bytes().ct_eq(key.as_bytes()))
                    && url::Url::parse(&input.config.url)
                        .is_ok_and(|url| url.path().trim_matches('/') == app)
            })
            .cloned()
            .ok_or("Invalid source token")?;
        input
            .connected
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "This source already has a publisher")?;
        match Publisher::start(input.clone(), parent) {
            Ok(publisher) => Ok(publisher),
            Err(error) => {
                input.connected.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    pub async fn stop(&self) {
        let formats = std::mem::take(&mut *self.formats.lock().await);
        for format in &formats {
            format.cancel.cancel();
        }
        for format in formats {
            format.join().await;
        }
    }
    pub async fn close(&self) {
        self.shutdown.cancel();
        let _control = self.control.lock().await;
        self.stop().await;
        for worker in self.clear_inputs() {
            let _ = worker.await;
        }
    }

    fn clear_inputs(&self) -> Vec<InputWorker> {
        self.source_previews.lock().unwrap().clear();
        self.inputs
            .lock()
            .unwrap()
            .drain()
            .filter_map(|(_, input)| {
                input.disconnect();
                input.stop_managed()
            })
            .collect()
    }
}

pub(crate) fn validate(workspace: &Workspace) -> Result<(), String> {
    let mut layout = workspace.clone();
    layout.horizontal.outputs.clear();
    layout.vertical.outputs.clear();
    layout.validate()?;
    workspace.validate_audio()?;
    if workspace.sources.len() > 8 || workspace.scenes.iter().any(|s| s.layers.len() > 32) {
        return Err("At most 8 sources and 32 layers per scene are supported".into());
    }
    let mut endpoints = std::collections::HashSet::new();
    for source in workspace.sources.iter().filter(|s| s.kind.is_rtmp()) {
        let url = url::Url::parse(&source.url).map_err(|_| "Invalid source RTMP URL")?;
        let app = url.path().trim_matches('/');
        if !matches!(url.scheme(), "rtmp" | "rtmps")
            || url.host_str().is_none()
            || app.is_empty()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
        {
            return Err(
                "Source URL must be rtmp:// or rtmps://host:port/application; put its token in Stream key"
                    .into(),
            );
        }
        if !endpoints.insert((app.to_owned(), source.key.clone())) {
            return Err("Each source needs a unique token for its RTMP application".into());
        }
    }
    for stream in [&workspace.horizontal, &workspace.vertical] {
        if u64::from(stream.base.width) * u64::from(stream.base.height) > 3840 * 2160 {
            return Err(
                "The live compositor supports at most 3840 × 2160 pixels (portrait also supported)"
                    .into(),
            );
        }
    }

    if workspace.horizontal.outputs.len() + workspace.vertical.outputs.len() > 8 {
        return Err("At most 8 outputs are supported".into());
    }
    Ok(())
}

pub(crate) fn validate_destination(
    output: &Output,
    capabilities: &MediaCapabilities,
) -> Result<(), String> {
    validate_destination_settings(output)?;
    if !capabilities.available {
        return Err("FFmpeg is unavailable".into());
    }
    if !capabilities
        .output_codecs(output.protocol)
        .contains(&output.codec)
    {
        return Err(format!(
            "No {} output encoder for {}",
            output.protocol.label(),
            output.codec
        ));
    }
    capabilities.resolve_video_encoder(&output.codec, output.encoder.as_deref())?;
    Ok(())
}

fn validate_destination_settings(output: &Output) -> Result<(), String> {
    output.validate()?;
    crate::output::destination_url(output)?;
    if u64::from(output.resolution.width) * u64::from(output.resolution.height) > 3840 * 2160 {
        return Err("Output resolution exceeds 3840 × 2160 pixels".into());
    }
    Ok(())
}

fn invalid(error: String) -> (StatusCode, Json<ApiError>) {
    (
        if error.starts_with("conflict:") {
            StatusCode::CONFLICT
        } else if error.contains("save workspace") {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        },
        Json(ApiError { error }),
    )
}

pub async fn status(State(runtime): State<RuntimeState>) -> Json<LiveStatus> {
    Json(runtime.status().await)
}

pub async fn edit_mixer(
    State(runtime): State<RuntimeState>,
    Json(request): Json<MixerRequest>,
) -> Result<Json<MixerAck>, (StatusCode, Json<ApiError>)> {
    if request.edits.is_empty() || request.edits.len() > MAX_MIXER_EDITS {
        return Err(invalid("Invalid number of mixer edits".into()));
    }
    let _guard = runtime.control.lock().await;
    let revision = runtime
        .update_workspace_checked(
            &request.workspace_id,
            Some(request.expected_revision),
            |workspace| {
                for edit in &request.edits {
                    edit.apply(workspace)?;
                }
                Ok(())
            },
        )
        .map_err(invalid)?;
    Ok(Json(MixerAck { revision }))
}

pub async fn save_source(
    State(runtime): State<RuntimeState>,
    Json(request): Json<SourceSettingsRequest>,
) -> Result<Json<SourceSettingsAck>, (StatusCode, Json<ApiError>)> {
    let _guard = runtime.control.lock().await;
    let revision = runtime
        .update_workspace_checked(
            &request.workspace_id,
            Some(request.expected_revision),
            |workspace| request.edit.apply(workspace, request.source_id).map(|_| ()),
        )
        .map_err(invalid)?;
    if request.edit.renews_capture() {
        runtime.refresh_source_capture(request.source_id).await;
    }
    Ok(Json(SourceSettingsAck { revision }))
}

pub async fn save_scene(
    State(runtime): State<RuntimeState>,
    Json(request): Json<SceneSettingsRequest>,
) -> Result<Json<SceneSettingsAck>, (StatusCode, Json<ApiError>)> {
    let _guard = runtime.control.lock().await;
    let running = runtime
        .formats
        .lock()
        .await
        .iter()
        .map(|format| format.orientation)
        .collect::<Vec<_>>();
    let revision = runtime.update_workspace_checked(
        &request.workspace_id, Some(request.expected_revision), |workspace| {
            if matches!(request.edit, SceneEdit::Remove)
                && running.iter().any(|orientation| workspace.active(*orientation) == Some(request.scene_id))
            {
                return Err("conflict: Switch to another scene or stop this format before removing its program scene.".into());
            }
            request.edit.apply(workspace, request.scene_id)
        },
    ).map_err(invalid)?;
    Ok(Json(SceneSettingsAck { revision }))
}

pub async fn save_output_profile(
    State(runtime): State<RuntimeState>,
    Json(request): Json<OutputProfileRequest>,
) -> Result<Json<OutputProfileAck>, (StatusCode, Json<ApiError>)> {
    let _guard = runtime.control.lock().await;
    let capabilities = runtime.capabilities.lock().await.clone();
    let mut formats = runtime.formats.lock().await;
    let target = request.target;
    let running = formats
        .iter_mut()
        .find(|format| format.orientation == target.orientation);
    let mut encoder_changed = false;
    let revision = runtime
        .update_workspace_checked(
            &request.workspace_id,
            Some(request.expected_revision),
            |workspace| {
                let previous = workspace
                    .stream(target.orientation)
                    .outputs
                    .get(target.index)
                    .cloned()
                    .ok_or("Output must be saved before editing its profile")?;
                request.apply(workspace)?;
                let stream = workspace.stream(target.orientation);
                let output = &stream.outputs[target.index];
                validate_destination_settings(output)?;
                encoder_changed = !output.same_encoder_config(&previous);
                if encoder_changed && capabilities.available {
                    validate_destination(output, &capabilities)?;
                    let fps = running
                        .as_ref()
                        .map_or(stream.fps, |format| format.spec.fps);
                    crate::output::validate_video_config(output, fps, &capabilities)?;
                }
                Ok(())
            },
        )
        .map_err(invalid)?;
    if encoder_changed && let Some(format) = running {
        format.invalidate_output_preview(target.index);
    }
    Ok(Json(OutputProfileAck { revision }))
}

pub async fn take_scene(
    State(runtime): State<RuntimeState>,
    Json(request): Json<TakeSceneRequest>,
) -> Result<Json<TakeSceneAck>, (StatusCode, Json<ApiError>)> {
    let _guard = runtime.control.lock().await;
    check_owner(&runtime, &request.workspace_id)?;
    if !runtime
        .formats
        .lock()
        .await
        .iter()
        .any(|format| format.orientation == request.orientation)
    {
        return Err(invalid(
            "Start this format before switching its program scene".into(),
        ));
    }
    let revision = runtime
        .update_workspace_checked(
            &request.workspace_id,
            Some(request.expected_revision),
            |workspace| request.apply(workspace),
        )
        .map_err(invalid)?;
    Ok(Json(TakeSceneAck { revision }))
}

pub async fn configure(
    State(runtime): State<RuntimeState>,
    Json(workspace): Json<Workspace>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    runtime.apply(workspace).await.map_err(invalid)?;
    Ok(Json(runtime.status().await))
}
pub async fn start(
    State(runtime): State<RuntimeState>,
    Json(request): Json<StartRequest>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    if !runtime.formats.lock().await.is_empty() {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "Already running; stop before starting again".into(),
            }),
        ));
    }
    if request.formats.is_empty() || request.formats.len() > 2 {
        return Err(invalid("Choose at least one scene/format".into()));
    }
    if let Some(target) = request.output
        && (request.preview_only || !request.formats.contains(&target.orientation))
    {
        return Err(invalid("Choose an output in a requested format".into()));
    }
    let mut seen = std::collections::HashSet::new();
    let capabilities: MediaCapabilities = runtime.capabilities.lock().await.clone();
    if !capabilities.available && !request.preview_only {
        return Err(invalid("FFmpeg is unavailable".into()));
    }
    for orientation in &request.formats {
        if !seen.insert(orientation.label()) || request.workspace.active(*orientation).is_none() {
            return Err(invalid("Select one scene for each requested format".into()));
        }
        let outputs = &request.workspace.stream(*orientation).outputs;
        if let Some(target) = request.output {
            if target.orientation == *orientation {
                let output = outputs
                    .get(target.index)
                    .filter(|output| output.enabled)
                    .ok_or_else(|| invalid("This destination is unavailable or disabled".into()))?;
                validate_destination(output, &capabilities).map_err(invalid)?;
            }
        } else if !request.preview_only {
            for output in outputs.iter().filter(|output| output.enabled) {
                validate_destination(output, &capabilities).map_err(invalid)?;
            }
        }
    }
    runtime.apply(request.workspace).await.map_err(invalid)?;
    for orientation in request.formats {
        let format = crate::compositor::start(
            runtime.clone(),
            orientation,
            &capabilities,
            request.preview_only || request.output.is_some(),
        )
        .await;
        match format {
            Ok(format) => runtime.formats.lock().await.push(format),
            Err(error) => {
                runtime.stop().await;
                return Err(invalid(error));
            }
        }
    }
    if let Some(target) = request.output {
        let outputs = current_outputs(&runtime, target.orientation);
        let mut formats = runtime.formats.lock().await;
        if let Some(format) = formats
            .iter_mut()
            .find(|format| format.orientation == target.orientation)
        {
            // The destination status owns encoder errors so the preview remains available.
            format
                .run_outputs(Some(target.index), &outputs, &capabilities, false)
                .await;
        }
    }
    Ok(Json(runtime.status().await))
}

fn check_owner(runtime: &RuntimeState, id: &str) -> Result<(), (StatusCode, Json<ApiError>)> {
    if runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|w| w.id != id)
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "Another workspace owns the active session".into(),
            }),
        ));
    }
    Ok(())
}
pub async fn stop(
    State(runtime): State<RuntimeState>,
    Json(request): Json<ControlRequest>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    check_owner(&runtime, &request.workspace_id)?;
    runtime.stop().await;
    Ok(Json(runtime.status().await))
}
fn current_outputs(
    runtime: &Runtime,
    orientation: acheflow_media_api::workspace::Orientation,
) -> Vec<Output> {
    runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .map(|workspace| workspace.stream(orientation).outputs.clone())
        .unwrap_or_default()
}

pub async fn restart_output(
    State(runtime): State<RuntimeState>,
    Json(request): Json<RestartOutputRequest>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    check_owner(&runtime, &request.workspace_id)?;
    let capabilities = runtime.capabilities.lock().await.clone();
    {
        let mut formats = runtime.formats.lock().await;
        let format = formats
            .iter_mut()
            .find(|f| f.orientation == request.orientation)
            .ok_or_else(|| invalid("This format is not running".into()))?;
        let outputs = current_outputs(&runtime, request.orientation);
        let output = outputs
            .get(request.index)
            .ok_or_else(|| invalid("This destination is unavailable or disabled".into()))?;
        format
            .restart_output(request.index, output, &capabilities)
            .await
            .map_err(invalid)?;
    }
    Ok(Json(runtime.status().await))
}
pub async fn manage_outputs(
    State(runtime): State<RuntimeState>,
    Json(request): Json<ManageOutputsRequest>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    check_owner(&runtime, &request.workspace_id)?;
    let capabilities = runtime.capabilities.lock().await.clone();
    {
        let mut formats = runtime.formats.lock().await;
        if formats.is_empty() {
            return Err(invalid(
                "Start the preview before controlling outputs".into(),
            ));
        }
        let restart = matches!(request.action, OutputAction::Restart);
        if let Some(target) = request.output {
            let format = formats
                .iter_mut()
                .find(|format| {
                    format.orientation == target.orientation && format.has_output(target.index)
                })
                .ok_or_else(|| invalid("This destination is unavailable or disabled".into()))?;
            format
                .run_outputs(
                    Some(target.index),
                    &current_outputs(&runtime, target.orientation),
                    &capabilities,
                    restart,
                )
                .await;
        } else {
            futures_util::future::join_all(formats.iter_mut().map(|format| {
                let outputs = current_outputs(&runtime, format.orientation);
                let capabilities = &capabilities;
                async move {
                    format
                        .run_outputs(None, &outputs, capabilities, restart)
                        .await
                }
            }))
            .await;
        }
    }
    Ok(Json(runtime.status().await))
}

pub async fn deactivate(
    State(runtime): State<RuntimeState>,
    Json(request): Json<ControlRequest>,
) -> ApiResult {
    let _guard = runtime.control.lock().await;
    check_owner(&runtime, &request.workspace_id)?;
    runtime.stop().await;
    for worker in runtime.clear_inputs() {
        let _ = worker.await;
    }
    runtime.activated.store(false, Ordering::Release);
    runtime.control_revision.fetch_add(1, Ordering::AcqRel);
    Ok(Json(runtime.status().await))
}

pub async fn preview(
    State(runtime): State<RuntimeState>,
    Path(format): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let orientation = match format.as_str() {
        "horizontal.jpg" => acheflow_media_api::workspace::Orientation::Horizontal,
        "vertical.jpg" => acheflow_media_api::workspace::Orientation::Vertical,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let output = match query
        .get("output")
        .map(|index| index.parse::<usize>())
        .transpose()
    {
        Ok(output) => output,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let (encoded, snapshot_format) = match query.get("mode").map(String::as_str) {
        Some("encoded") if output.is_some() => (
            Some(runtime.capabilities.lock().await.clone()),
            SnapshotFormat::Jpeg,
        ),
        Some("lossless") => (None, SnapshotFormat::Png),
        None => (None, SnapshotFormat::Jpeg),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let (frames, resolution, cache) = {
        let mut formats = runtime.formats.lock().await;
        let profile = {
            let workspace = runtime.workspace.read().unwrap();
            let workspace = workspace.as_ref();
            if query
                .get("workspace")
                .is_some_and(|id| workspace.is_none_or(|w| &w.id != id))
            {
                return StatusCode::NOT_FOUND.into_response();
            }
            output.and_then(|index| workspace?.stream(orientation).outputs.get(index).cloned())
        };
        let Some(format) = formats.iter_mut().find(|f| f.orientation == orientation) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let (Some(index), Some(capabilities)) = (output, encoded.as_ref()) {
            let Some(profile) = profile else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let view = format.encoded_view(index, profile, capabilities);
            // No compositor or workspace lock is needed to serve a snapshot.
            drop(formats);
            return match view {
                Ok(view) => view.response().await,
                Err(status) => status.into_response(),
            };
        }
        let resolution = if output.is_some() {
            let Some(profile) = profile else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let Some(size) = preview_size(&profile.resolution) else {
                return StatusCode::UNPROCESSABLE_ENTITY.into_response();
            };
            Some(size)
        } else {
            None
        };
        (
            format.frame.clone(),
            resolution,
            format.preview_cache.clone(),
        )
    };
    cache
        .lock()
        .await
        .response(&frames, resolution, snapshot_format)
        .await
}

fn preview_size(size: &Resolution) -> Option<Resolution> {
    if !(1..=7680).contains(&size.width) || !(1..=7680).contains(&size.height) {
        return None;
    }
    let pixels = f64::from(size.width) * f64::from(size.height);
    let scale = (8_294_400. / pixels).sqrt().min(1.);
    Some(Resolution {
        width: (f64::from(size.width) * scale).floor().max(1.) as u32,
        height: (f64::from(size.height) * scale).floor().max(1.) as u32,
    })
}
pub async fn renew_source_preview(
    State(runtime): State<RuntimeState>,
    Json(request): Json<SourcePreviewRequest>,
) -> Result<Json<()>, (StatusCode, Json<ApiError>)> {
    runtime
        .renew_source_preview(request)
        .await
        .map_err(invalid)?;
    Ok(Json(()))
}

pub async fn source_preview(
    State(runtime): State<RuntimeState>,
    Path(id): Path<u32>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let frame = {
        let workspace = runtime.workspace.read().unwrap();
        if query
            .get("workspace")
            .is_some_and(|id| workspace.as_ref().is_none_or(|w| &w.id != id))
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        runtime.inputs.lock().unwrap().get(&id).and_then(|input| {
            if !input.connected.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            input
                .frame
                .read()
                .unwrap()
                .as_ref()
                .filter(|f| f.received.elapsed() < std::time::Duration::from_secs(2))
                .map(|f| f.image.clone())
        })
    };
    image_response(frame, None).await
}
async fn image_response(
    frame: Option<Arc<image::RgbImage>>,
    resolution: Option<Resolution>,
) -> Response {
    let Some(frame) = frame else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match encode_preview(frame, resolution, SnapshotFormat::Jpeg).await {
        Some(snapshot) => snapshot.response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SnapshotFormat {
    Jpeg,
    Png,
}

struct PreviewSnapshot {
    format: SnapshotFormat,
    bytes: axum::body::Bytes,
    width: u32,
    height: u32,
}

impl PreviewSnapshot {
    fn response(&self) -> Response {
        let mut response = (
            [
                (
                    header::CONTENT_TYPE,
                    match self.format {
                        SnapshotFormat::Jpeg => "image/jpeg",
                        SnapshotFormat::Png => "image/png",
                    },
                ),
                (header::CACHE_CONTROL, "no-store"),
            ],
            self.bytes.clone(),
        )
            .into_response();
        response
            .headers_mut()
            .insert("x-frame-width", self.width.into());
        response
            .headers_mut()
            .insert("x-frame-height", self.height.into());
        response
    }
}

#[derive(Default)]
pub(crate) struct PreviewCache {
    frame: Weak<image::RgbImage>,
    snapshots: VecDeque<PreviewSnapshot>,
}

impl PreviewCache {
    async fn response(
        &mut self,
        frames: &tokio::sync::watch::Receiver<Arc<image::RgbImage>>,
        resolution: Option<Resolution>,
        format: SnapshotFormat,
    ) -> Response {
        // Read the current frame after waiting for an in-progress encode. Slow
        // clients then skip old frames instead of building up an encode backlog.
        let frame = frames.borrow().clone();
        let resolution = resolution.or_else(|| {
            if format != SnapshotFormat::Png {
                return None;
            }
            preview_size(&Resolution {
                width: frame.width(),
                height: frame.height(),
            })
        });
        let dimensions = resolution
            .as_ref()
            .map_or_else(|| frame.dimensions(), |size| (size.width, size.height));
        let identity = Arc::downgrade(&frame);
        if !self.frame.ptr_eq(&identity) {
            self.snapshots.clear();
            self.frame = identity;
        }
        if let Some(snapshot) = self.snapshots.iter().find(|snapshot| {
            snapshot.format == format && (snapshot.width, snapshot.height) == dimensions
        }) {
            return snapshot.response();
        }
        let Some(snapshot) = encode_preview(frame, resolution, format).await else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let response = snapshot.response();
        if self.snapshots.len() == 4 {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
        response
    }
}

async fn encode_preview(
    frame: Arc<image::RgbImage>,
    resolution: Option<Resolution>,
    format: SnapshotFormat,
) -> Option<PreviewSnapshot> {
    tokio::task::spawn_blocking(move || {
        let resized = resolution
            .filter(|size| frame.dimensions() != (size.width, size.height))
            .map(|size| {
                image::imageops::resize(
                    frame.as_ref(),
                    size.width,
                    size.height,
                    image::imageops::FilterType::CatmullRom,
                )
            });
        let frame = resized.as_ref().unwrap_or(frame.as_ref());
        let bytes = match format {
            SnapshotFormat::Jpeg => {
                let mut bytes = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
                    .encode_image(frame)?;
                bytes.into()
            }
            SnapshotFormat::Png => crate::encoded_preview::encode_png(frame)?,
        };
        Ok::<_, image::ImageError>(PreviewSnapshot {
            format,
            bytes,
            width: frame.width(),
            height: frame.height(),
        })
    })
    .await
    .ok()?
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn scoped_scene_save_keeps_program_sources_and_existing_mixer() {
        use acheflow_media_api::{live::SceneContent, workspace::Orientation};
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_source();
        let program = workspace.add_scene(Orientation::Horizontal);
        let other = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(other, workspace.sources[0].id);
        workspace.scenes[1].layers[0].width = 640;
        workspace.scenes[1].audio.buses[0].gain = 81;
        runtime.apply(workspace.clone()).await.unwrap();
        let mut draft = workspace.clone();
        draft.set_active(Orientation::Horizontal, Some(other));
        draft.sources[0].name = "A source edit kept in the draft".into();
        draft.scenes[0].name = "A different scene edit".into();
        draft.scenes[1].layers[0].x = 55;
        draft.scenes[1].audio.buses[0].gain = 33;

        let Json(ack) = save_scene(
            State(runtime.clone()),
            Json(SceneSettingsRequest {
                workspace_id: workspace.id.clone(),
                expected_revision: runtime.saved_state().0,
                scene_id: other,
                edit: SceneEdit::Content(SceneContent::of(&draft.scenes[1])),
            }),
        )
        .await
        .unwrap();
        let (revision, stored) = runtime.saved_state();
        let stored = stored.unwrap();
        assert_eq!(revision, ack.revision);
        assert_eq!(stored.active(Orientation::Horizontal), Some(program));
        assert_eq!(stored.sources, workspace.sources);
        assert_eq!(stored.scenes[0].name, workspace.scenes[0].name);
        assert_eq!(stored.scenes[1].layers[0].x, 55);
        assert_eq!(stored.scenes[1].audio.buses[0].gain, 81);

        let new_scene = draft.add_scene(Orientation::Horizontal);
        let Json(_) = save_scene(
            State(runtime.clone()),
            Json(SceneSettingsRequest {
                workspace_id: workspace.id,
                expected_revision: ack.revision,
                scene_id: new_scene,
                edit: SceneEdit::Register(draft.scenes.pop().unwrap()),
            }),
        )
        .await
        .unwrap();
        let stored = runtime.saved_state().1.unwrap();
        assert_eq!(stored.active(Orientation::Horizontal), Some(program));
        assert!(stored.scenes.iter().any(|scene| scene.id == new_scene));
    }
    #[tokio::test]
    async fn scoped_source_settings_preserve_unrelated_inputs_and_program() {
        use acheflow_media_api::{live::SourceEdit, workspace::Orientation};
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_source();
        workspace.add_source();
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene, 2);
        runtime.apply(workspace.clone()).await.unwrap();
        let first = runtime.inputs.lock().unwrap()[&1].clone();
        let second = runtime.inputs.lock().unwrap()[&2].clone();
        let first_connection = CancellationToken::new();
        let second_connection = CancellationToken::new();
        *first.cancel.lock().unwrap() = Some(first_connection.clone());
        *second.cancel.lock().unwrap() = Some(second_connection.clone());
        let request = |source_id, edit| SourceSettingsRequest {
            workspace_id: workspace.id.clone(),
            expected_revision: runtime.saved_state().0,
            source_id,
            edit,
        };

        let Json(_) = save_source(
            State(runtime.clone()),
            Json(request(2, SourceEdit::Name("Renamed".into()))),
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&second, &runtime.inputs.lock().unwrap()[&2]));
        assert!(!second_connection.is_cancelled());

        let mut capture = SourceCapture::of(&workspace.sources[1]);
        capture.key = acheflow_media_api::workspace::new_token();
        let Json(_) = save_source(
            State(runtime.clone()),
            Json(request(2, SourceEdit::Capture(capture.clone()))),
        )
        .await
        .unwrap();
        assert!(second_connection.is_cancelled());
        assert!(capture.matches(&runtime.inputs.lock().unwrap()[&2].config));
        assert!(Arc::ptr_eq(&first, &runtime.inputs.lock().unwrap()[&1]));
        assert!(!first_connection.is_cancelled());

        let Json(_) = save_source(State(runtime.clone()), Json(request(2, SourceEdit::Remove)))
            .await
            .unwrap();
        assert!(!runtime.inputs.lock().unwrap().contains_key(&2));
        let saved = runtime.saved_state().1.unwrap();
        assert_eq!(saved.active(Orientation::Horizontal), Some(scene));
        assert!(saved.scenes[0].layers.is_empty());
        assert!(!first_connection.is_cancelled());
    }

    #[tokio::test]
    async fn adding_a_source_does_not_activate_an_inactive_workspace() {
        use acheflow_media_api::live::SourceEdit;
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        runtime.install(workspace.clone(), false).await.unwrap();
        workspace.add_source();
        let Json(_) = save_source(
            State(runtime.clone()),
            Json(SourceSettingsRequest {
                workspace_id: workspace.id,
                expected_revision: runtime.saved_state().0,
                source_id: 1,
                edit: SourceEdit::Register(workspace.sources.remove(0)),
            }),
        )
        .await
        .unwrap();
        assert_eq!(runtime.saved_state().1.unwrap().sources.len(), 1);
        assert!(!runtime.activated.load(Ordering::Acquire));
        assert!(runtime.inputs.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn mixer_updates_are_scoped_to_audio_and_validate_the_whole_batch() {
        use acheflow_media_api::{
            audio::{BusChange, MixerEdit, StripChange, TrackKey},
            workspace::Orientation,
        };
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_source();
        let program = workspace.add_scene(Orientation::Horizontal);
        let draft = workspace.add_scene(Orientation::Horizontal);
        workspace.set_active(Orientation::Horizontal, Some(program));
        runtime.apply(workspace.clone()).await.unwrap();
        let revision = runtime.saved_state().0;
        let gain = MixerEdit::Strip {
            scene: program,
            key: TrackKey {
                source_id: workspace.sources[0].id,
                track_id: 0,
            },
            change: StripChange::Gain(137),
        };
        let request = |expected_revision, edits| MixerRequest {
            workspace_id: workspace.id.clone(),
            expected_revision,
            edits,
        };
        let Json(ack) = edit_mixer(
            State(runtime.clone()),
            Json(request(revision, vec![gain.clone()])),
        )
        .await
        .unwrap();
        let mix = runtime.saved_state().1.unwrap();
        assert_eq!(mix.active(Orientation::Horizontal), Some(program));
        assert_eq!(
            mix.scenes
                .iter()
                .find(|s| s.id == program)
                .unwrap()
                .audio
                .strips[0]
                .gain,
            137
        );
        assert!(
            mix.scenes
                .iter()
                .find(|s| s.id == draft)
                .unwrap()
                .audio
                .strips
                .is_empty()
        );
        assert_eq!(
            edit_mixer(State(runtime.clone()), Json(request(revision, vec![gain])))
                .await
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        let rejected = edit_mixer(
            State(runtime.clone()),
            Json(request(
                ack.revision,
                vec![
                    MixerEdit::Bus {
                        scene: program,
                        bus: 1,
                        change: BusChange::Mute(true),
                    },
                    MixerEdit::Bus {
                        scene: draft,
                        bus: 1,
                        change: BusChange::Gain(300),
                    },
                ],
            )),
        )
        .await;
        assert_eq!(rejected.unwrap_err().0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(runtime.saved_state(), (ack.revision, Some(mix)));
    }
    #[tokio::test]
    async fn preview_session_keeps_enabled_destination_without_starting_encoder() {
        use acheflow_media_api::workspace::{Orientation, Output, Resolution};

        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.horizontal.base = Resolution {
            width: 320,
            height: 240,
        };
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.set_active(Orientation::Horizontal, Some(scene));
        workspace.horizontal.outputs.push(Output {
            protocol: Default::default(),
            name: "Preview test".into(),
            url: "rtmp://127.0.0.1:1/live".into(),
            key: "preview-test".into(),
            resolution: Resolution {
                width: 160,
                height: 90,
            },
            quality: "Custom".into(),
            codec: "h264".into(),
            encoder: None,
            bitrate_kbps: 2000,
            bitrate_mode: Default::default(),
            keyframe_interval_secs: 2,
            enabled: true,
            audio: Default::default(),
        });
        let workspace_id = workspace.id.clone();
        let Json(status) = start(
            State(runtime.clone()),
            Json(StartRequest {
                workspace,
                formats: vec![Orientation::Horizontal],
                preview_only: true,
                output: None,
            }),
        )
        .await
        .unwrap();
        assert!(
            runtime
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .horizontal
                .outputs[0]
                .enabled
        );
        assert_eq!(
            status.formats[0].outputs[0].state,
            acheflow_media_api::live::OutputState::Stopped
        );
        assert_eq!(status.formats[0].preview_profiles[0].index, 0);
        assert_eq!(status.formats[0].applied_scene.as_ref().unwrap().id, scene);
        let response = preview(
            State(runtime.clone()),
            Path("horizontal.jpg".into()),
            Query(HashMap::from([
                ("workspace".into(), workspace_id),
                ("output".into(), "0".into()),
            ])),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-frame-width"], "160");
        let outside_format = preview(
            State(runtime.clone()),
            Path("horizontal.jpg".into()),
            Query(HashMap::from([("output".into(), "1".into())])),
        )
        .await;
        assert_eq!(outside_format.status(), StatusCode::NOT_FOUND);
        runtime.stop().await;
        assert!(runtime.status().await.formats.is_empty());
    }
    #[tokio::test]
    async fn output_framing_preview_uses_the_profile_dimensions() {
        use acheflow_media_api::workspace::Resolution;
        let target = Resolution {
            width: 320,
            height: 180,
        };
        assert_eq!(preview_size(&target), Some(target.clone()));
        assert_eq!(
            preview_size(&Resolution {
                width: 7680,
                height: 4320
            }),
            Some(Resolution {
                width: 3840,
                height: 2160
            })
        );
        let response =
            image_response(Some(Arc::new(image::RgbImage::new(640, 480))), Some(target)).await;
        assert_eq!(response.headers()["x-frame-width"], "320");
        assert_eq!(response.headers()["x-frame-height"], "180");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            image::load_from_memory(&bytes)
                .unwrap()
                .to_rgb8()
                .dimensions(),
            (320, 180)
        );
    }
    #[tokio::test]
    async fn format_snapshot_cache_reuses_the_frame_and_expires_on_publish() {
        let (tx, rx) = tokio::sync::watch::channel(Arc::new(image::RgbImage::new(32, 24)));
        let mut cache = PreviewCache::default();
        assert_eq!(
            cache
                .response(&rx, None, SnapshotFormat::Jpeg)
                .await
                .status(),
            StatusCode::OK
        );
        let previous = cache.snapshots[0].bytes.clone();
        let response = cache.response(&rx, None, SnapshotFormat::Jpeg).await;
        let reused = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        assert_eq!(previous.as_ptr(), reused.as_ptr());
        tx.send_replace(Arc::new(image::RgbImage::from_pixel(
            32,
            24,
            image::Rgb([255, 255, 255]),
        )));
        assert_eq!(
            cache
                .response(&rx, None, SnapshotFormat::Jpeg)
                .await
                .status(),
            StatusCode::OK
        );
        assert_ne!(previous, cache.snapshots[0].bytes);
        assert_eq!(cache.snapshots.len(), 1);
    }
    #[tokio::test]
    async fn lossless_preview_preserves_every_rgb_pixel_with_a_separate_cache_entry() {
        let image = Arc::new(image::RgbImage::from_fn(64, 32, |x, y| {
            image::Rgb([(x * 3) as u8, (y * 7) as u8, (x * y) as u8])
        }));
        let (_tx, rx) = tokio::sync::watch::channel(image.clone());
        let mut cache = PreviewCache::default();
        let jpeg = cache.response(&rx, None, SnapshotFormat::Jpeg).await;
        assert_eq!(jpeg.headers()["content-type"], "image/jpeg");
        let png = cache.response(&rx, None, SnapshotFormat::Png).await;
        assert_eq!(png.headers()["content-type"], "image/png");
        let bytes = axum::body::to_bytes(png.into_body(), 64 * 1024)
            .await
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded, *image);
        assert_eq!(cache.snapshots.len(), 2);
    }

    #[tokio::test]
    async fn jpeg_preview_preserves_native_detail_and_never_upscales() {
        for (width, height) in [(1920, 1080), (1080, 1920), (64, 32)] {
            let image = image::RgbImage::from_fn(width, height, |x, y| {
                let value = if (x / 3 + y / 3) % 2 == 0 { 32 } else { 224 };
                image::Rgb([value; 3])
            });
            let response = image_response(Some(Arc::new(image.clone())), None).await;
            assert_eq!(response.headers()["x-frame-width"], width.to_string());
            let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
                .await
                .unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
            assert_eq!(decoded.dimensions(), (width, height));
            let error: f64 = image
                .as_raw()
                .iter()
                .zip(decoded.as_raw())
                .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                .sum::<f64>()
                / image.as_raw().len() as f64;
            assert!(error < 25., "fine detail lost: MSE={error}");
        }
    }
    #[test]
    fn secure_ingest_urls_are_validated_like_plain_rtmp() {
        let mut workspace = Workspace::default();
        workspace.add_source();
        workspace.sources[0].url = "rtmps://localhost:1935/live".into();
        assert!(validate(&workspace).is_ok());
        workspace.sources[0].url = "https://localhost:1935/live".into();
        assert!(validate(&workspace).is_err());
        workspace.sources[0].url = "rtmps://localhost:1935".into();
        assert!(validate(&workspace).is_err());
    }
    #[tokio::test]
    async fn offline_preview_resize_does_not_replace_a_registered_input() {
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_source();
        runtime.apply(workspace.clone()).await.unwrap();
        let input = runtime.inputs.lock().unwrap()[&1].clone();
        workspace.sources[0].resolution = acheflow_media_api::workspace::Resolution {
            width: 64,
            height: 64,
        };
        workspace.edit_revision = runtime.saved_state().0;
        runtime.apply(workspace).await.unwrap();
        assert!(Arc::ptr_eq(&input, &runtime.inputs.lock().unwrap()[&1]));
    }
    #[tokio::test]
    async fn activating_sources_promotes_the_preview_input_without_disconnect() {
        let runtime = Runtime::new(Default::default(), "127.0.0.1:0".into());
        let mut workspace = Workspace::default();
        workspace.add_source();
        runtime.install(workspace.clone(), false).await.unwrap();
        let request = SourcePreviewRequest {
            workspace_id: workspace.id.clone(),
            source_id: 1,
        };
        runtime.renew_source_preview(request).await.unwrap();
        let input = runtime.inputs.lock().unwrap()[&1].clone();

        workspace.edit_revision = runtime.saved_state().0;
        runtime.install(workspace, true).await.unwrap();
        assert!(runtime.source_previews.lock().unwrap().is_empty());
        runtime.clone().expire_source_preview(input.clone()).await;
        assert!(Arc::ptr_eq(&input, &runtime.inputs.lock().unwrap()[&1]));
        assert!(runtime.activated.load(Ordering::Acquire));
        runtime.close().await;
    }

    #[tokio::test]
    async fn source_snapshots_disappear_on_disconnect_and_stall_and_return_on_reconnect() {
        let runtime = Runtime::new(
            Arc::new(AsyncMutex::new(MediaCapabilities::default())),
            "127.0.0.1:0".into(),
        );
        let mut workspace = Workspace::default();
        workspace.add_source();
        let input = Arc::new(Input::new(workspace.sources[0].clone()));
        runtime
            .inputs
            .lock()
            .unwrap()
            .insert(input.config.id, input.clone());
        let snapshot = || {
            source_preview(
                State(runtime.clone()),
                Path(input.config.id),
                Query(HashMap::new()),
            )
        };
        let image = Arc::new(image::RgbImage::new(16, 16));
        *input.frame.write().unwrap() = Some(crate::ingest::VideoFrame {
            pts: None,
            generation: 0,
            received: std::time::Instant::now(),
            image: image.clone(),
        });
        assert_eq!(snapshot().await.status(), StatusCode::NOT_FOUND);
        input.connected.store(true, Ordering::Release);
        let response = snapshot().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-frame-width"], "16");
        assert_eq!(response.headers()["x-frame-height"], "16");
        input.frame.write().unwrap().as_mut().unwrap().received =
            std::time::Instant::now() - std::time::Duration::from_secs(3);
        assert_eq!(snapshot().await.status(), StatusCode::NOT_FOUND);
        input.frame.write().unwrap().as_mut().unwrap().received = std::time::Instant::now();
        assert_eq!(snapshot().await.status(), StatusCode::OK);
        input.connected.store(false, Ordering::Release);
        assert_eq!(snapshot().await.status(), StatusCode::NOT_FOUND);
    }
    #[test]
    fn source_registration_rejects_shared_tokens_and_non_rtmp_urls() {
        let mut w = Workspace::default();
        w.add_source();
        w.add_source();
        w.sources[1].key = w.sources[0].key.clone();
        assert!(validate(&w).is_err());
        w.sources[1].key = acheflow_media_api::workspace::new_token();
        assert!(validate(&w).is_ok());
        w.sources[0].url = "file:///etc/passwd".into();
        assert!(validate(&w).is_err());
    }
}
