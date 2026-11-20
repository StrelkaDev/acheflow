//! Device authentication is required even for requests arriving over loopback.
mod direct_actions;
mod packages;
mod personal_accounts;
mod policy;
mod registry;
mod server_accounts;
use acheflow_media_api::{
    access::{Devices, InviteRequest, PairRequest, Permission as P, ProfileRequest},
    control::ControlRequest,
};
use axum::{
    Extension, Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
pub(crate) use policy::Principal;
use registry::{Error, Registry};
use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf, sync::Arc};

const COOKIE: &str = "__Host-acheflow_session";
pub(crate) struct Access {
    registry: Registry,
    host: Arc<crate::control::ControlHost>,
    origins: BTreeSet<String>,
    account_checks: tokio::sync::Semaphore,
    package_checks: Arc<tokio::sync::Semaphore>,
    pub(crate) package_store: Arc<acheflow_client::package::Store>,
}
impl Access {
    pub fn server_id(&self) -> String {
        self.registry.server_id()
    }
    pub fn open(
        path: PathBuf,
        host: Arc<crate::control::ControlHost>,
    ) -> Result<Arc<Self>, String> {
        let package_root = path.with_extension("packages");
        let registry = Registry::open(path).map_err(|e| e.1.to_owned())?;
        let origins = std::env::var("ACHEFLOW_UI_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| url::Url::parse(v).map(|url| url.origin().ascii_serialization()))
            .collect::<Result<_, _>>()
            .map_err(|_| "Invalid UI origin.".to_owned())?;
        let package_store = Arc::new(acheflow_client::package::Store::open(
            package_root,
            acheflow_extension_api::parts::ExecutionHost::Server,
        )?);
        for package in package_store.installed()? {
            host.replace_package(package);
        }
        *host.packages.lock().unwrap() = Some(package_store.clone());
        Ok(Arc::new(Self {
            registry,
            package_store,
            package_checks: Arc::new(tokio::sync::Semaphore::new(1)),
            host,
            origins,
            account_checks: tokio::sync::Semaphore::new(2),
        }))
    }
    pub fn routes(self: &Arc<Self>) -> Router {
        Router::new()
            .route(
                "/api/extensions/server-providers",
                get(server_accounts::catalog),
            )
            .route(
                "/api/extensions/server-providers/{provider}/module",
                get(server_accounts::module),
            )
            .route(
                "/api/access/actions/authorize",
                post(direct_actions::authorize),
            )
            .route("/api/extensions/packages", get(packages::list))
            .route(
                "/api/extensions/packages/stage",
                post(packages::stage).layer(axum::extract::DefaultBodyLimit::max(
                    acheflow_client::package::MAX_ARCHIVE_BYTES,
                )),
            )
            .route("/api/extensions/packages/install", post(packages::install))
            .route("/api/extensions/packages/allow", post(packages::allow))
            .route(
                "/api/access/personal-send/{workspace}/{provider}/{channel}",
                post(personal_accounts::retired),
            )
            .route(
                "/api/access/personal-accounts/{workspace}",
                get(personal_accounts::list),
            )
            .route(
                "/api/access/personal-accounts/{workspace}/{provider}",
                post(personal_accounts::retired).delete(personal_accounts::disconnect),
            )
            .route("/api/access/pair", post(pair))
            .route("/api/access/session", get(session))
            .route("/api/access/invitations", post(invite))
            .route("/api/access/link-invitations", post(link_invite))
            .route("/api/access/my-devices", get(my_devices))
            .route("/api/access/profile", put(profile))
            .route("/api/access/devices", get(devices))
            .route("/api/access/devices/{id}", delete(revoke))
            .route("/api/access/logout", post(logout))
            .route(
                "/api/access/server-accounts/{provider}",
                get(server_accounts::settings)
                    .post(server_accounts::connect)
                    .put(server_accounts::configure)
                    .delete(server_accounts::disconnect),
            )
            .with_state(self.clone())
    }
    pub fn protect(self: &Arc<Self>, app: Router) -> Router {
        app.layer(middleware::from_fn_with_state(self.clone(), authorize))
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut response = (
            StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(serde_json::json!({"error":{"code":"access_denied","message":self.1}})),
        )
            .into_response();
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        response
    }
}
fn denied() -> Error {
    Error(403, "This device does not have permission.")
}
fn now() -> u64 {
    crate::control::now_ms()
}
fn supplied(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        return value.to_str().ok()?.strip_prefix("Bearer ");
    }
    let mut matching = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE);
    let (_, token) = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(token)
}
fn origin_allowed(headers: &HeaderMap, trusted: &BTreeSet<String>) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if trusted.contains(origin) {
        return true;
    }
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let authority = &origin[origin.find("://").map(|n| n + 3).unwrap_or(0)..];
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    authority == host && (url.scheme() == "https" || (loopback && url.scheme() == "http"))
}
async fn authorize(
    State(access): State<Arc<Access>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Error> {
    let path = request.uri().path().to_owned();
    if !path.starts_with("/api/") {
        return Ok(next.run(request).await);
    }
    if !origin_allowed(request.headers(), &access.origins) {
        return Err(Error(403, "This browser origin is not allowed."));
    }
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }
    if request.method() == Method::GET && path == "/api/server/info" {
        return Ok(next.run(request).await);
    }
    let mutation = request.method() != Method::GET && request.method() != Method::HEAD;
    if mutation
        && request
            .headers()
            .get("x-acheflow-request")
            .and_then(|h| h.to_str().ok())
            != Some("1")
    {
        return Err(Error(403, "A same-origin API request is required."));
    }
    if path == "/api/access/pair" && request.method() == Method::POST {
        return Ok(next.run(request).await);
    }
    let token = supplied(request.headers())
        .ok_or(Error(401, "Pair this device with the server."))?
        .to_owned();
    let mut principal = Principal(
        access
            .registry
            .authenticate(&token, now())
            .ok_or(Error(401, "Device session is invalid or expired."))?,
    );
    if mutation && path == "/api/extensions/packages/stage" {
        if !principal.allows(P::ExtensionsUpload) {
            return Err(Error(403, "This device cannot upload extension packages."));
        }
        let permit = access
            .package_checks
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error(429, "Another package operation is in progress."))?;
        request.extensions_mut().insert(Arc::new(permit));
    }
    let body = if mutation {
        let (parts, body) = request.into_parts();
        let limit = if path == "/api/extensions/packages/stage" {
            if !principal.allows(P::ExtensionsUpload) {
                return Err(Error(403, "This device cannot upload extension packages."));
            }
            acheflow_client::package::MAX_ARCHIVE_BYTES
        } else {
            1024 * 1024
        };
        let bytes = if path == "/api/extensions/packages/stage" {
            tokio::time::timeout(std::time::Duration::from_secs(30), to_bytes(body, limit))
                .await
                .map_err(|_| Error(408, "Package upload timed out."))?
                .map_err(|_| Error(413, "Request exceeds limit."))?
        } else {
            to_bytes(body, limit)
                .await
                .map_err(|_| Error(413, "Request exceeds limit."))?
        };
        // A slow body must not carry an expired/revoked session into a handler.
        principal = Principal(
            access
                .registry
                .authenticate(&token, now())
                .ok_or(Error(401, "Device session is invalid or expired."))?,
        );
        request = Request::from_parts(parts, Body::from(bytes.clone()));
        Some(bytes)
    } else {
        None
    };
    let allowed = match (request.method(), path.as_str()) {
        (&Method::GET, "/api/extensions/server-providers") => {
            principal.allows(P::ExtensionsConfigure)
        }
        (&Method::GET, path) if path.starts_with("/api/extensions/server-providers/") => {
            principal.server_accounts()
        }
        (&Method::POST, "/api/access/actions/authorize") => principal.allows(P::ChatSend),
        (&Method::GET, "/api/extensions/packages") => {
            principal.any(&[P::ExtensionsUpload, P::ExtensionsInstall, P::ExtensionsRun])
        }
        (&Method::POST, "/api/extensions/packages/stage") => principal.allows(P::ExtensionsUpload),
        (&Method::POST, "/api/extensions/packages/install") => {
            principal.allows(P::ExtensionsInstall)
        }
        (&Method::POST, "/api/extensions/packages/allow") => principal.allows(P::ExtensionsRun),
        (&Method::GET | &Method::PUT, path) if path.starts_with("/api/access/server-accounts/") => {
            principal.allows(P::ExtensionsConfigure)
        }
        (&Method::POST | &Method::DELETE, path)
            if path.starts_with("/api/access/server-accounts/") =>
        {
            principal.server_accounts()
        }
        (&Method::GET | &Method::POST | &Method::DELETE, path)
            if path.starts_with("/api/access/personal-accounts/") =>
        {
            true
        }
        (&Method::POST, path) if path.starts_with("/api/access/personal-send/") => {
            principal.allows(P::ChatSend)
        }
        (&Method::GET, "/api/workspace") => true,
        (&Method::POST, "/api/workspace/import") => principal.workspace_management(),
        (&Method::GET, "/api/access/session" | "/api/access/my-devices")
        | (&Method::POST, "/api/access/logout" | "/api/access/link-invitations")
        | (&Method::PUT, "/api/access/profile") => true,
        (&Method::GET, "/api/access/devices") | (&Method::POST, "/api/access/invitations") => {
            principal.allows(P::AccessManage)
        }
        (&Method::DELETE, path) if path.starts_with("/api/access/devices/") => {
            true // Registry enforces ownership or administrative authority.
        }
        (&Method::GET, "/api/server/settings" | "/api/server/logs")
        | (&Method::PUT, "/api/server/settings") => principal.allows(P::AccessManage),
        (&Method::GET, "/api/media/capabilities" | "/api/live/status") => {
            principal.any(&[P::StudioView, P::ScenesManage, P::MediaManage])
        }
        (
            &Method::POST,
            "/api/media/capabilities" | "/api/live/stop" | "/api/live/output/restart",
        ) => principal.allows(P::MediaManage),
        (&Method::GET, path)
            if path.starts_with("/api/live/preview/") || path.starts_with("/api/live/source/") =>
        {
            principal.allows(P::StudioView)
        }
        (&Method::GET, "/api/control/state") => principal.any(&[
            P::StudioView,
            P::ChatRead,
            P::ChatSend,
            P::InteractiveView,
            P::InteractiveManage,
            P::ScenesManage,
            P::ExtensionsConfigure,
            P::ExtensionsRun,
        ]),
        (&Method::PUT, "/api/live/config") | (&Method::POST, "/api/live/start") => {
            principal.all(&[
                P::WorkspaceManage,
                P::ScenesManage,
                P::MediaManage,
                P::ExtensionsConfigure,
                P::ExtensionsRun,
            ])
        }
        (&Method::DELETE, "/api/live/config") => {
            principal.all(&[P::WorkspaceManage, P::MediaManage, P::ExtensionsRun])
        }
        (&Method::POST, "/api/control/executor") => principal.allows(P::ExtensionsRun),
        (&Method::POST, "/api/control/commands") => {
            let command: ControlRequest =
                serde_json::from_slice(body.as_deref().unwrap_or_default())
                    .map_err(|_| Error(400, "Invalid control command."))?;
            principal.command(&command.action)
        }
        _ => false,
    };
    if !allowed {
        return Err(denied());
    }
    request.extensions_mut().insert(principal.clone());
    let mut response = next.run(request).await;
    if response.status().is_success()
        && matches!(
            path.as_str(),
            "/api/control/state"
                | "/api/control/commands"
                | "/api/live/status"
                | "/api/live/config"
                | "/api/live/start"
                | "/api/live/stop"
                | "/api/live/output/restart"
        )
    {
        let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .map_err(|_| Error(500, "State response exceeds limit."))?;
        response = if path.starts_with("/api/control/") {
            let state = serde_json::from_slice(&bytes)
                .map_err(|_| Error(500, "Invalid state response."))?;
            Json(principal.snapshot(state)).into_response()
        } else {
            let state =
                serde_json::from_slice(&bytes).map_err(|_| Error(500, "Invalid live response."))?;
            Json(principal.live(state)).into_response()
        };
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}
async fn pair(
    State(access): State<Arc<Access>>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    Json(request): Json<PairRequest>,
) -> Result<Response, Error> {
    let peer = peer
        .ok()
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|| "local".into());
    let mut result = access
        .registry
        .pair(&request.code, &request.name, &peer, now())?;
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=2592000",
        result.token.as_deref().unwrap_or_default()
    );
    if headers
        .get("x-acheflow-client")
        .and_then(|h| h.to_str().ok())
        != Some("native")
    {
        result.token = None;
    }
    let mut response = Json(result).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|_| Error(500, "Cannot create device session."))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}
async fn session(
    Extension(principal): Extension<Principal>,
) -> Json<acheflow_media_api::access::Session> {
    Json(principal.0)
}
async fn invite(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<InviteRequest>,
) -> Result<Json<acheflow_media_api::access::Invitation>, Error> {
    access
        .registry
        .invite(&principal.0, request.permissions, now())
        .map(Json)
}
async fn link_invite(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<InviteRequest>,
) -> Result<Json<acheflow_media_api::access::Invitation>, Error> {
    access
        .registry
        .link_invite(&principal.0, request.permissions, now())
        .map(Json)
}
async fn profile(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<ProfileRequest>,
) -> Result<Json<acheflow_media_api::access::Session>, Error> {
    access
        .registry
        .profile(&principal.0, &request.name, now())
        .map(Json)
}
async fn my_devices(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Devices>, Error> {
    Ok(Json(Devices {
        devices: access.registry.my_devices(&principal.0, now())?,
    }))
}
async fn devices(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Devices>, Error> {
    Ok(Json(Devices {
        devices: access.registry.devices(&principal.0, now())?,
    }))
}
async fn revoke(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    access.registry.revoke(&principal.0, &id, now())?;
    withdraw(&access, &id).await;
    Ok(StatusCode::NO_CONTENT)
}
async fn logout(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, Error> {
    access
        .registry
        .revoke(&principal.0, &principal.0.device.id, now())?;
    withdraw(&access, &principal.0.device.id).await;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
            .parse()
            .unwrap(),
    );
    Ok(response)
}
async fn withdraw(access: &Access, id: &str) {
    let _command = access.host.command_lock.lock().await;
    let workspace = access
        .host
        .runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .map(|w| w.id.clone());
    let Some(workspace) = workspace else {
        return;
    };
    let mut registry = access.host.client_execution.lock().unwrap();
    let actors = registry.connections(&workspace, now());
    for actor in actors
        .into_iter()
        .filter(|a| a.client_id.starts_with(&format!("studio-{id}-")))
    {
        let _ = registry.unregister(&workspace, &actor.client_id, actor.generation, now());
    }
}
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod tests;
