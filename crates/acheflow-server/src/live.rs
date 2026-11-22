use crate::{
    compositor::RunningFormat,
    ingest::{Input, Publisher},
};
use acheflow_media_api::{
    MediaCapabilities,
    live::{ApiError, ControlRequest, LiveStatus, RestartOutputRequest, StartRequest},
    workspace::Workspace,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock, atomic::Ordering},
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
        let workspace_id = self
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .map(|w| w.id.clone());
        let sources = self
            .inputs
            .lock()
            .unwrap()
            .values()
            .map(|i| i.status())
            .collect();
        let formats = self
            .formats
            .lock()
            .await
            .iter()
            .map(RunningFormat::status)
            .collect();
        LiveStatus {
            activated: self.activated.load(Ordering::Acquire),
            edit_revision: self
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .map(|w| w.edit_revision)
                .unwrap_or(0),
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

fn validate_outputs(
    workspace: &Workspace,
    formats: &[acheflow_media_api::workspace::Orientation],
) -> Result<(), String> {
    let mut execution = workspace.clone();
    for orientation in acheflow_media_api::workspace::Orientation::ALL {
        if formats.contains(&orientation) {
            execution
                .stream_mut(orientation)
                .outputs
                .retain(|o| o.enabled);
        } else {
            execution.stream_mut(orientation).outputs.clear();
        }
    }
    execution.validate()?;
    for orientation in formats {
        for output in execution.stream(*orientation).outputs.iter() {
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
            if u64::from(output.resolution.width) * u64::from(output.resolution.height)
                > 3840 * 2160
            {
                return Err("Output resolution exceeds 3840 × 2160 pixels".into());
            }
        }
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
    let mut seen = std::collections::HashSet::new();
    validate_outputs(&request.workspace, &request.formats).map_err(invalid)?;
    let capabilities: MediaCapabilities = runtime.capabilities.lock().await.clone();
    for orientation in &request.formats {
        if !seen.insert(orientation.label()) || request.workspace.active(*orientation).is_none() {
            return Err(invalid("Select one scene for each requested format".into()));
        }
        for output in request
            .workspace
            .stream(*orientation)
            .outputs
            .iter()
            .filter(|o| o.enabled)
        {
            if !capabilities.rtmp_codecs().contains(&output.codec) {
                return Err(invalid(format!("No RTMP encoder for {}", output.codec)));
            }
            capabilities
                .resolve_video_encoder(&output.codec, output.encoder.as_deref())
                .map_err(invalid)?;
        }
    }
    if !capabilities.available {
        return Err(invalid("FFmpeg is unavailable".into()));
    }
    runtime.apply(request.workspace).await.map_err(invalid)?;
    for orientation in request.formats {
        let format = crate::compositor::start(runtime.clone(), orientation, &capabilities).await;
        match format {
            Ok(format) => runtime.formats.lock().await.push(format),
            Err(error) => {
                runtime.stop().await;
                return Err(invalid(error));
            }
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
    let orientation = match format.as_str() {
        "horizontal.jpg" => acheflow_media_api::workspace::Orientation::Horizontal,
        "vertical.jpg" => acheflow_media_api::workspace::Orientation::Vertical,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let frame = runtime
        .formats
        .lock()
        .await
        .iter()
        .find(|f| f.orientation == orientation)
        .map(|f| f.frame.borrow().clone());
    image_response(frame).await
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
    image_response(frame).await
}
async fn image_response(frame: Option<Arc<image::RgbImage>>) -> Response {
    let Some(frame) = frame else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (width, height) = frame.dimensions();
    let result = tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode_image(frame.as_ref())
            .map(|()| bytes)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => {
            let mut response = (
                [
                    (header::CONTENT_TYPE, "image/jpeg"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response();
            // Dimensions of this exact frame, without rescaling the JPEG preview.
            response.headers_mut().insert("x-frame-width", width.into());
            response
                .headers_mut()
                .insert("x-frame-height", height.into());
            response
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn jpeg_preview_preserves_native_detail_and_never_upscales() {
        for (width, height) in [(1920, 1080), (1080, 1920), (64, 32)] {
            let image = image::RgbImage::from_fn(width, height, |x, y| {
                let value = if (x / 3 + y / 3) % 2 == 0 { 32 } else { 224 };
                image::Rgb([value; 3])
            });
            let response = image_response(Some(Arc::new(image.clone()))).await;
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
