use crate::{
    compositor::RunningFormat,
    ingest::{Input, Publisher},
};
use acheflow_media_api::{
    MediaCapabilities,
    live::{
        ApiError, ControlRequest, LiveStatus, ManageOutputsRequest, OutputAction,
        RestartOutputRequest, StartRequest,
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

struct InputChanges {
    retired: Vec<InputWorker>,
    managed: Vec<Arc<Input>>,
}

pub struct Runtime {
    pub visual_services: RwLock<Option<Arc<Mutex<crate::control::Services>>>>,
    pub control_revision: std::sync::atomic::AtomicU64,
    pub workspace: RwLock<Option<Workspace>>,
    pub inputs: Mutex<HashMap<u32, Arc<Input>>>,
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
                format.status(scene)
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
        let mut current = self.workspace.write().unwrap();
        let mut next = current
            .as_ref()
            .filter(|w| w.id == owner)
            .cloned()
            .ok_or("conflict: workspace changed")?;
        update(&mut next)?;
        validate(&next)?;
        self.commit(&mut current, Some(next))
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
            use acheflow_media_api::visual::SourceKind;
            match &input.config.kind {
                SourceKind::Srt { port } => match self.rtmp_address.parse::<std::net::SocketAddr>()
                {
                    Ok(mut address) if srt_available => {
                        address.set_port(*port);
                        crate::srt_input::start(&input, address, &self.shutdown);
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
                    crate::camera_input::start(&input, &self.shutdown);
                }
                SourceKind::Microphone { .. } => {
                    crate::microphone_input::start(&input, &self.shutdown);
                }
                _ => {}
            }
        }
        Ok(())
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
        self.stop().await;
        for worker in self.clear_inputs() {
            let _ = worker.await;
        }
    }

    fn clear_inputs(&self) -> Vec<InputWorker> {
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
    output.validate()?;
    let url = url::Url::parse(&output.url).map_err(|_| "Invalid output RTMP URL")?;
    if url.scheme() != "rtmp"
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
    {
        return Err("Invalid output RTMP URL".into());
    }
    if output.key.len() > 256 || output.key.chars().any(char::is_control) {
        return Err("Invalid output key".into());
    }
    if u64::from(output.resolution.width) * u64::from(output.resolution.height) > 3840 * 2160 {
        return Err("Output resolution exceeds 3840 × 2160 pixels".into());
    }
    if !capabilities.available {
        return Err("FFmpeg is unavailable".into());
    }
    if !capabilities.rtmp_codecs().contains(&output.codec) {
        return Err(format!("No RTMP encoder for {}", output.codec));
    }
    capabilities.resolve_video_encoder(&output.codec, output.encoder.as_deref())?;
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
        let mut formats = runtime.formats.lock().await;
        if let Some(format) = formats
            .iter_mut()
            .find(|format| format.orientation == target.orientation)
        {
            // The destination status owns encoder errors so the preview remains available.
            format
                .run_outputs(Some(target.index), &capabilities, false)
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
        format
            .restart_output(request.index, &capabilities)
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
                .run_outputs(Some(target.index), &capabilities, restart)
                .await;
        } else {
            futures_util::future::join_all(
                formats
                    .iter_mut()
                    .map(|format| format.run_outputs(None, &capabilities, restart)),
            )
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
    let mode = match query.get("mode").map(String::as_str) {
        Some("encoded") if output.is_some() => Some(runtime.capabilities.lock().await.clone()),
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
        None => None,
    };
    let (frames, resolution, cache) = {
        let mut formats = runtime.formats.lock().await;
        let encoded_profile = {
            let workspace = runtime.workspace.read().unwrap();
            let workspace = workspace.as_ref();
            if query
                .get("workspace")
                .is_some_and(|id| workspace.is_none_or(|w| &w.id != id))
            {
                return StatusCode::NOT_FOUND.into_response();
            }
            mode.as_ref()
                .and_then(|_| workspace?.stream(orientation).outputs.get(output?).cloned())
        };
        let Some(format) = formats.iter_mut().find(|f| f.orientation == orientation) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let (Some(index), Some(capabilities)) = (output, mode.as_ref()) {
            let Some(profile) = encoded_profile else {
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
        let resolution = if let Some(index) = output {
            let Some(profile) = format
                .preview_profiles
                .iter()
                .find(|profile| profile.index == index)
            else {
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
    cache.lock().await.response(&frames, resolution).await
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
pub async fn source_preview(
    State(runtime): State<RuntimeState>,
    Path(id): Path<u32>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if query.get("workspace").is_some_and(|id| {
        runtime
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .is_none_or(|w| &w.id != id)
    }) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let frame = runtime.inputs.lock().unwrap().get(&id).and_then(|input| {
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
    });
    image_response(frame, None).await
}
async fn image_response(
    frame: Option<Arc<image::RgbImage>>,
    resolution: Option<Resolution>,
) -> Response {
    let Some(frame) = frame else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match encode_preview(frame, resolution).await {
        Some(snapshot) => snapshot.response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

struct PreviewSnapshot {
    bytes: axum::body::Bytes,
    width: u32,
    height: u32,
}

impl PreviewSnapshot {
    fn response(&self) -> Response {
        let mut response = (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
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
    ) -> Response {
        // Read the current frame after waiting for an in-progress encode. Slow
        // clients then skip old frames instead of building up an encode backlog.
        let frame = frames.borrow().clone();
        let dimensions = resolution
            .as_ref()
            .map_or_else(|| frame.dimensions(), |size| (size.width, size.height));
        let identity = Arc::downgrade(&frame);
        if !self.frame.ptr_eq(&identity) {
            self.snapshots.clear();
            self.frame = identity;
        }
        if let Some(snapshot) = self
            .snapshots
            .iter()
            .find(|snapshot| (snapshot.width, snapshot.height) == dimensions)
        {
            return snapshot.response();
        }
        let Some(snapshot) = encode_preview(frame, resolution).await else {
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
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode_image(frame)
            .map(|()| PreviewSnapshot {
                bytes: bytes.into(),
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
        assert_eq!(cache.response(&rx, None).await.status(), StatusCode::OK);
        let previous = cache.snapshots[0].bytes.clone();
        let response = cache.response(&rx, None).await;
        let reused = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        assert_eq!(previous.as_ptr(), reused.as_ptr());
        tx.send_replace(Arc::new(image::RgbImage::from_pixel(
            32,
            24,
            image::Rgb([255, 255, 255]),
        )));
        assert_eq!(cache.response(&rx, None).await.status(), StatusCode::OK);
        assert_ne!(previous, cache.snapshots[0].bytes);
        assert_eq!(cache.snapshots.len(), 1);
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
