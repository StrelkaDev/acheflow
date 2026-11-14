//! Saved project API; connecting a client does not import or activate a project.
use crate::{access::Principal, control::ControlHost};
use acheflow_media_api::{
    live::ApiError,
    server_workspace::{ImportWorkspace, WorkspaceState, WorkspaceSummary},
};
use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use std::sync::Arc;

pub(crate) fn routes(host: Arc<ControlHost>) -> Router {
    Router::new()
        .route("/api/workspace", get(snapshot))
        .route("/api/workspace/import", post(import))
        .with_state(host)
}
fn view(host: &ControlHost, principal: &Principal) -> WorkspaceState {
    let (revision, workspace) = host.runtime.saved_state();
    let summary = workspace.as_ref().map(|w| WorkspaceSummary {
        id: w.id.clone(),
        name: w.name.clone(),
    });
    let editable = principal.workspace_management();
    let view = if editable {
        None
    } else {
        workspace.clone().map(|w| display(w, principal))
    };
    WorkspaceState {
        server_id: principal.0.server_id.clone(),
        revision,
        summary,
        view,
        workspace: if editable { workspace } else { None },
    }
}
async fn snapshot(
    State(host): State<Arc<ControlHost>>,
    Extension(principal): Extension<Principal>,
) -> Json<WorkspaceState> {
    Json(view(&host, &principal))
}
type Error = (StatusCode, Json<ApiError>);
fn error(error: String) -> Error {
    let status = if error.starts_with("conflict:") {
        StatusCode::CONFLICT
    } else if error.contains("save workspace") {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    (status, Json(ApiError { error }))
}
async fn import(
    State(host): State<Arc<ControlHost>>,
    Extension(principal): Extension<Principal>,
    Json(mut request): Json<ImportWorkspace>,
) -> Result<Json<WorkspaceState>, Error> {
    let _command = host.command_lock.lock().await;
    if !host.guests.lock().await.is_empty() || host.providers.active().await {
        return Err(error(
            "conflict: Deactivate extensions before importing a workspace.".into(),
        ));
    }
    crate::workspace_extensions::validate_public_settings(&host, &request.workspace)
        .map_err(error)?;
    let previous = host
        .runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .map(|w| w.id.clone());
    request.workspace.edit_revision = request.expected_revision;
    let _control = host.runtime.control.lock().await;
    host.runtime
        .install(request.workspace, false)
        .await
        .map_err(error)?;
    if let Some(previous) = previous {
        host.client_execution
            .lock()
            .unwrap()
            .withdraw_workspace(&previous, crate::control::now_ms());
    }
    Ok(Json(view(&host, &principal)))
}

fn display(
    mut workspace: acheflow_media_api::workspace::Workspace,
    principal: &Principal,
) -> acheflow_media_api::workspace::Workspace {
    use acheflow_media_api::access::Permission as P;
    workspace.server_url.clear();
    workspace.client_extensions.clear();
    workspace.extensions.clear();
    workspace.client_capabilities.clear();
    workspace.server_integrations.clear();
    for source in &mut workspace.sources {
        source.url.clear();
        source.key.clear();
    }
    for stream in [&mut workspace.horizontal, &mut workspace.vertical] {
        for output in &mut stream.outputs {
            output.url.clear();
            output.key.clear();
        }
    }
    if !principal.any(&[P::StudioView, P::ScenesManage, P::InteractiveView]) {
        workspace.sources.clear();
        workspace.scenes.clear();
        workspace.horizontal.outputs.clear();
        workspace.vertical.outputs.clear();
        workspace.horizontal_scene = None;
        workspace.vertical_scene = None;
    }
    workspace
}
