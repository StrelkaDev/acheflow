use super::tests::Directory;
use super::*;
use acheflow_media_api::{
    access::{PairResponse, Session},
    control::{ControlAction, ControlSnapshot},
    live::LiveStatus,
};
use axum::{
    body::{Body, to_bytes},
    http::Request as HttpRequest,
    routing::put,
};
use tower::ServiceExt;

fn fixture() -> (Directory, Arc<Access>, Router, String) {
    let directory = Directory::new();
    let (host, _) = crate::control::tests::host();
    let access = Access::open(directory.path(), host).unwrap();
    let owner = directory.owner();
    let snapshot = || async {
        let mut snapshot = ControlSnapshot {
            server_providers: vec![acheflow_media_api::server_provider::ServerProviderStatus {
                provider: "fixture".into(),
                server_owned: true,
                account_id: Some("private-account-fixture".into()),
                ..Default::default()
            }],
            client_executors: vec![acheflow_extension_api::client_execution::ExecutorStatus {
                client_id: "server-fixture".into(),
                generation: 1,
                expires_at_ms: u64::MAX,
                connections: vec![acheflow_extension_api::client_execution::Connection {
                    channel_label: None,
                    stream_id: None,
                    id: "binding".into(),
                    provider: "fixture".into(),
                    account_id: "private-account-fixture".into(),
                    channel_id: "channel".into(),
                    capabilities: Default::default(),
                }],
            }],
            ..Default::default()
        };
        snapshot.extension_settings.insert(
            "fixture".into(),
            acheflow_media_api::workspace::ExtensionSettings {
                enabled: true,
                config: std::collections::BTreeMap::from([(
                    "private-config".into(),
                    "fixture-value".into(),
                )]),
            },
        );
        snapshot
            .chat
            .entries
            .push(acheflow_extension_api::interaction::ChatEntry {
                cursor: 1,
                deleted: false,
                merged_into: None,
                delivery: vec![],
                message: acheflow_extension_api::interaction::ChatMessage {
                    platform: "fixture".into(),
                    channel_id: "channel".into(),
                    message_id: "message".into(),
                    user_id: "user".into(),
                    display_name: "Fixture".into(),
                    text: "private-chat-fixture".into(),
                    roles: vec![],
                    occurred_at_ms: 0,
                },
            });
        Json(snapshot)
    };
    let app = Router::new()
        .merge(access.routes())
        .merge(crate::server_workspace::routes(access.host.clone()))
        .route(
            "/api/live/status",
            get(|| async { Json(LiveStatus::default()) }),
        )
        .route("/api/control/state", get(snapshot))
        .route("/api/control/commands", post(snapshot))
        .route(
            "/api/live/config",
            put(|| async { Json(LiveStatus::default()) }),
        )
        .route(
            "/api/live/outputs",
            post(crate::live::manage_outputs).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/mixer",
            post(crate::live::edit_mixer).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/take",
            post(crate::live::take_scene).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/source-preview",
            post(crate::live::renew_source_preview).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/sources",
            post(crate::live::save_source).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/output/profile",
            post(crate::live::save_output_profile).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/live/scenes",
            post(crate::live::save_scene).with_state(access.host.runtime.clone()),
        )
        .route(
            "/api/control/executor",
            post(crate::client_bridge::studio).with_state(access.host.clone()),
        )
        .route("/api/server/settings", get(|| async { "private-settings" }))
        .route("/api/server/logs", get(|| async { "private-log" }));
    (directory, access.clone(), access.protect(app), owner)
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: serde_json::Value,
    origin: Option<&str>,
) -> Response {
    let mut request = HttpRequest::builder()
        .method(method)
        .uri(path)
        .header("host", "studio.example")
        .header("content-type", "application/json")
        .header("x-acheflow-request", "1")
        .header("x-acheflow-client", "native");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
async fn value(response: Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
async fn paired(app: &Router, owner: &str, permissions: &[P]) -> PairResponse {
    let invitation = value(
        call(
            app,
            "POST",
            "/api/access/invitations",
            Some(owner),
            serde_json::json!({"permissions":permissions}),
            None,
        )
        .await,
    )
    .await;
    let response = call(
        app,
        "POST",
        "/api/access/pair",
        None,
        serde_json::json!({"code":invitation["code"],"name":"Phone fixture"}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_value(value(response).await).unwrap()
}

#[tokio::test]
async fn source_preview_requires_media_control_permission() {
    let (_directory, access, app, owner) = fixture();
    let mut workspace = acheflow_media_api::workspace::Workspace::default();
    workspace.add_source();
    let request = serde_json::json!({"workspace_id": workspace.id, "source_id": 1});
    let runtime = &access.host.runtime;
    runtime.install(workspace, false).await.unwrap();
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    let path = "/api/live/source-preview";
    let refused = call(
        &app,
        "POST",
        path,
        viewer.token.as_deref(),
        request.clone(),
        None,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(runtime.inputs.lock().unwrap().is_empty());
    let allowed = call(&app, "POST", path, Some(&owner), request, None).await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert!(runtime.inputs.lock().unwrap().contains_key(&1));
    runtime.close().await;
}

#[tokio::test]
async fn saving_an_output_profile_returns_the_revision_acknowledgment() {
    use acheflow_media_api::{
        live::{OutputProfile, OutputProfileRequest, OutputTarget},
        workspace::{Orientation, Workspace},
    };
    let (_directory, access, app, owner) = fixture();
    let mut workspace = Workspace::default();
    let profile: OutputProfile = serde_json::from_value(serde_json::json!({
        "name": "Test destination", "url": "rtmp://127.0.0.1/live", "key": "fixture",
        "resolution": {"width": 320, "height": 180}, "quality": "Standard",
        "codec": "h264", "encoder": null, "bitrate_kbps": 1000,
        "bitrate_mode": "vbr", "keyframe_interval_secs": 2
    }))
    .unwrap();
    let mut output = serde_json::to_value(&profile).unwrap();
    output["enabled"] = false.into();
    workspace
        .horizontal
        .outputs
        .push(serde_json::from_value(output).unwrap());
    let runtime = &access.host.runtime;
    let status = crate::live::configure(
        axum::extract::State(runtime.clone()),
        Json(workspace.clone()),
    )
    .await
    .unwrap()
    .0;
    let request = OutputProfileRequest {
        workspace_id: workspace.id,
        expected_revision: status.edit_revision,
        target: OutputTarget {
            orientation: Orientation::Horizontal,
            index: 0,
        },
        profile: OutputProfile {
            name: "Renamed destination".into(),
            ..profile
        },
    };
    let body = serde_json::to_value(request).unwrap();
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    let route = "/api/live/output/profile";
    let denied = call(
        &app,
        "POST",
        route,
        viewer.token.as_deref(),
        body.clone(),
        None,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let response = call(&app, "POST", route, Some(&owner), body, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        value(response).await,
        serde_json::json!({"revision": status.edit_revision + 1})
    );
    let current = runtime.workspace.read().unwrap();
    let saved = &current.as_ref().unwrap().horizontal.outputs[0];
    assert_eq!(saved.name, "Renamed destination");
    assert!(!saved.enabled);
}

#[tokio::test]
async fn every_api_boundary_requires_authentication_even_with_loopback_proxy_headers() {
    let (_directory, _access, app, _owner) = fixture();
    for (method, path) in [
        ("GET", "/api/live/status"),
        ("GET", "/api/control/state"),
        ("GET", "/api/server/settings"),
        ("GET", "/api/server/logs"),
        ("PUT", "/api/live/config"),
        ("POST", "/api/live/start"),
        ("POST", "/api/live/stop"),
        ("POST", "/api/live/output/restart"),
        ("POST", "/api/live/mixer"),
        ("POST", "/api/live/take"),
        ("POST", "/api/live/sources"),
        ("POST", "/api/live/scenes"),
        ("POST", "/api/live/output/profile"),
        ("POST", "/api/live/outputs"),
        ("GET", "/api/live/preview/horizontal.jpg"),
        ("GET", "/api/live/source/1"),
        ("POST", "/api/live/source-preview"),
        ("POST", "/api/control/executor"),
        ("GET", "/api/access/devices"),
        ("POST", "/api/access/invitations"),
        ("POST", "/api/control/commands"),
        ("POST", "/api/future-route"),
    ] {
        let request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:21295")
            .header("x-acheflow-request", "1")
            .header("x-forwarded-for", "127.0.0.1")
            .header("x-forwarded-proto", "https")
            .body(Body::from("{}"))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn studio_and_chat_readers_get_only_their_granted_state_and_cannot_mutate() {
    let (_directory, _access, app, owner) = fixture();
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    let chat = paired(&app, &owner, &[P::ChatRead]).await;
    let viewer_token = viewer.token.as_deref().unwrap();
    let chat_token = chat.token.as_deref().unwrap();
    let studio = value(
        call(
            &app,
            "GET",
            "/api/control/state",
            Some(viewer_token),
            serde_json::Value::Null,
            None,
        )
        .await,
    )
    .await;
    assert!(studio["chat"]["entries"].as_array().unwrap().is_empty());
    assert_eq!(studio["extension_settings"], serde_json::json!({}));
    let chat_state = value(
        call(
            &app,
            "GET",
            "/api/control/state",
            Some(chat_token),
            serde_json::Value::Null,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(chat_state["chat"]["entries"].as_array().unwrap().len(), 1);
    assert!(chat_state["scenes"].as_array().unwrap().is_empty());
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/live/status",
            Some(chat_token),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    for (method, path) in [
        ("PUT", "/api/live/config"),
        ("POST", "/api/control/executor"),
        ("GET", "/api/server/settings"),
        ("GET", "/api/server/logs"),
    ] {
        assert_eq!(
            call(
                &app,
                method,
                path,
                Some(viewer_token),
                serde_json::json!({}),
                None
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn command_permissions_are_enforced_before_execution_and_response_projection() {
    let (_directory, _access, app, owner) = fixture();
    let sender = paired(&app, &owner, &[P::ChatSend]).await;
    let body = serde_json::json!({"workspace_id":"fixture","request_id":"fixture","expected_revision":0,"action":{"type":"send_chat","targets":[],"text":"fixture"}});
    let response = call(
        &app,
        "POST",
        "/api/control/commands",
        sender.token.as_deref(),
        body,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = serde_json::json!({"workspace_id":"fixture","request_id":"fixture","expected_revision":0,"action":{"type":"set_extension","id":"fixture","enabled":true}});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/control/commands",
            sender.token.as_deref(),
            body,
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let root_session: Session = serde_json::from_value(
        value(
            call(
                &app,
                "GET",
                "/api/access/session",
                Some(&owner),
                serde_json::Value::Null,
                None,
            )
            .await,
        )
        .await,
    )
    .unwrap();
    let scene = Principal(Session {
        device: acheflow_media_api::access::Device {
            permissions: [P::ScenesManage].into_iter().collect(),
            ..root_session.device.clone()
        },
        ..root_session.clone()
    });
    assert!(scene.command(&ControlAction::SetScene {
        scene_id: 1,
        spatial: acheflow_media_api::spatial::SpatialScene::from_canvas(
            &acheflow_media_api::workspace::Resolution {
                width: 16,
                height: 16
            }
        )
    }));
    assert!(!scene.command(&ControlAction::SetExtension {
        id: "fixture".into(),
        enabled: true,
        config: None
    }));
    assert!(!scene.command(&ControlAction::SetCallbackProxy {
        url: "https://callbacks.example".into()
    }));
    let configure = Principal(Session {
        device: acheflow_media_api::access::Device {
            permissions: [P::ExtensionsConfigure].into_iter().collect(),
            ..root_session.device.clone()
        },
        ..root_session
    });
    assert!(configure.command(&ControlAction::SetCallbackProxy {
        url: "https://callbacks.example".into()
    }));
}

#[tokio::test]
async fn browser_cookie_is_host_scoped_http_only_secure_and_csrf_checks_ignore_forwarded_headers() {
    let (_directory, _access, app, owner) = fixture();
    let invitation = value(
        call(
            &app,
            "POST",
            "/api/access/invitations",
            Some(&owner),
            serde_json::json!({"permissions":["studio_view"]}),
            None,
        )
        .await,
    )
    .await;
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/access/pair")
        .header("host", "studio.example")
        .header("origin", "https://studio.example")
        .header("x-acheflow-request", "1")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"name":"Browser","code":invitation["code"]}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    for flag in [
        "__Host-acheflow_session=",
        "Path=/",
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
    ] {
        assert!(cookie.contains(flag));
    }
    assert!(!cookie.contains("Domain="));
    assert!(value(response).await.get("token").is_none());
    let cookie = cookie.split(';').next().unwrap();
    let request = HttpRequest::builder()
        .uri("/api/access/session")
        .header("host", "studio.example")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/access/logout")
        .header("host", "studio.example")
        .header("origin", "https://evil.example")
        .header("x-forwarded-host", "evil.example")
        .header("x-forwarded-proto", "https")
        .header("cookie", cookie)
        .header("x-acheflow-request", "1")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/access/session",
            Some(&owner),
            serde_json::Value::Null,
            Some("http://studio.example")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn revocation_immediately_rejects_a_previously_valid_device_and_logout_works_for_viewers() {
    let (_directory, _access, app, owner) = fixture();
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    let path = format!("/api/access/devices/{}", viewer.session.device.id);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &path,
            Some(&owner),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/access/session",
            viewer.token.as_deref(),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/access/logout",
            viewer.token.as_deref(),
            serde_json::json!({}),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/access/session",
            viewer.token.as_deref(),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn executor_identity_is_device_bound_and_revocation_withdraws_its_registrations() {
    let (_directory, access, app, owner) = fixture();
    let first = paired(&app, &owner, &[P::ExtensionsRun]).await;
    let second = paired(&app, &owner, &[P::ExtensionsRun]).await;
    let workspace = {
        let mut saved = access.host.runtime.workspace.write().unwrap();
        let saved = saved.as_mut().unwrap();
        saved.client_capabilities = ["channel-a", "channel-b"]
            .into_iter()
            .map(
                |channel| acheflow_extension_api::client_execution::CapabilityGrant {
                    provider: "executor-fixture".into(),
                    account_id: "bot".into(),
                    channel_id: channel.into(),
                    capabilities: [acheflow_extension_api::client_execution::Capability::ChatSend]
                        .into_iter()
                        .collect(),
                },
            )
            .collect();
        saved.id.clone()
    };
    let body = serde_json::json!({"workspace_id":workspace,"client_id":"studio-same","operation":{"type":"register","registration":{"version":1,"generation":1,"connections":[{"id":"fixture","provider":"executor-fixture","account_id":"bot","channel_id":"channel-a","capabilities":["chat.send"]}]}}});
    for (index, device) in [&first, &second].into_iter().enumerate() {
        let mut body = body.clone();
        body["operation"]["registration"]["connections"][0]["channel_id"] =
            serde_json::json!(if index == 0 { "channel-a" } else { "channel-b" });
        let response = call(
            &app,
            "POST",
            "/api/control/executor",
            device.token.as_deref(),
            body.clone(),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(value(response).await["client_id"], "studio-same");
    }
    let actors = access
        .host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace, now());
    assert_eq!(actors.len(), 2);
    assert_ne!(actors[0].client_id, actors[1].client_id);
    let mut attack = body.clone();
    attack["operation"]["registration"]["generation"] = serde_json::json!(2);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/control/executor",
            second.token.as_deref(),
            attack,
            None
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let path = format!("/api/access/devices/{}", first.session.device.id);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &path,
            Some(&owner),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let remaining = access
        .host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace, now());
    assert_eq!(remaining.len(), 1);
    assert!(
        remaining[0]
            .client_id
            .starts_with(&format!("studio-{}-", second.session.device.id))
    );
}

#[tokio::test]
async fn a_slow_request_cannot_retain_authority_after_device_revocation() {
    let (_directory, _access, app, owner) = fixture();
    let sender = paired(&app, &owner, &[P::ChatSend]).await;
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let polled = Arc::new(tokio::sync::Notify::new());
    let body_polled = polled.clone();
    let body = Body::from_stream(futures_util::stream::once(async move {
        body_polled.notify_one();
        receive.await.unwrap();
        Ok::<_,std::io::Error>(axum::body::Bytes::from(serde_json::json!({"workspace_id":"fixture","request_id":"fixture","expected_revision":0,"action":{"type":"send_chat","targets":[],"text":"fixture"}}).to_string()))
    }));
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/control/commands")
        .header("host", "studio.example")
        .header("content-type", "application/json")
        .header("x-acheflow-request", "1")
        .header(
            "authorization",
            format!("Bearer {}", sender.token.as_deref().unwrap()),
        )
        .body(body)
        .unwrap();
    let pending = tokio::spawn(app.clone().oneshot(request));
    polled.notified().await;
    let path = format!("/api/access/devices/{}", sender.session.device.id);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &path,
            Some(&owner),
            serde_json::Value::Null,
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    send.send(()).unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn saved_configuration_is_private_and_import_requires_all_apply_rights() {
    let (_directory, _access, app, owner) = fixture();
    let viewer = paired(&app, &owner, &[P::StudioView]).await;
    let mut workspace = acheflow_media_api::workspace::Workspace::default();
    workspace.add_source();
    workspace.add_scene(acheflow_media_api::workspace::Orientation::Horizontal);
    workspace
        .horizontal
        .outputs
        .push(acheflow_media_api::workspace::Output {
            protocol: Default::default(),
            name: "Fixture destination".into(),
            url: "rtmp://destination.example/live".into(),
            key: "fixture-output-secret".into(),
            resolution: workspace.horizontal.base.clone(),
            quality: "Standard".into(),
            codec: "h264".into(),
            encoder: None,
            bitrate_kbps: 2500,
            bitrate_mode: Default::default(),
            keyframe_interval_secs: 2,
            enabled: false,
            audio: Default::default(),
        });
    let body =
        serde_json::json!({"expected_revision":0,"workspace":workspace.server_configuration()});
    let denied = call(
        &app,
        "POST",
        "/api/workspace/import",
        viewer.token.as_deref(),
        body.clone(),
        None,
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let response = call(
        &app,
        "POST",
        "/api/workspace/import",
        Some(&owner),
        body.clone(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let saved = value(response).await;
    assert_eq!(saved["workspace"]["id"], workspace.id);
    assert_eq!(saved["revision"], 1);
    let response = call(
        &app,
        "GET",
        "/api/workspace",
        viewer.token.as_deref(),
        serde_json::Value::Null,
        None,
    )
    .await;
    let filtered = value(response).await;
    assert!(filtered["workspace"].is_null());
    assert_eq!(filtered["view"]["sources"][0]["key"], "");
    assert_eq!(filtered["view"]["sources"][0]["url"], "");
    assert!(
        filtered["view"]["extensions"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(!filtered.to_string().contains(&workspace.sources[0].key));
    assert_eq!(filtered["view"]["horizontal"]["outputs"][0]["key"], "");
    assert_eq!(filtered["view"]["horizontal"]["outputs"][0]["url"], "");
    assert!(!filtered.to_string().contains("fixture-output-secret"));
    let reader = paired(&app, &owner, &[P::ChatRead]).await;
    let response = call(
        &app,
        "GET",
        "/api/workspace",
        reader.token.as_deref(),
        serde_json::Value::Null,
        None,
    )
    .await;
    let chat_only = value(response).await;
    assert!(chat_only["view"]["scenes"].as_array().unwrap().is_empty());
    assert!(chat_only["view"]["sources"].as_array().unwrap().is_empty());
    assert_eq!(filtered["summary"]["id"], workspace.id);
    let response = call(
        &app,
        "POST",
        "/api/workspace/import",
        Some(&owner),
        body,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn server_account_metadata_is_hidden_from_run_only_devices_in_every_snapshot_field() {
    let (_directory, _access, app, owner) = fixture();
    let runner = paired(&app, &owner, &[P::ChatRead, P::ExtensionsRun]).await;
    let response = call(
        &app,
        "GET",
        "/api/control/state",
        runner.token.as_deref(),
        serde_json::Value::Null,
        None,
    )
    .await;
    let snapshot = value(response).await;
    assert_eq!(snapshot["server_providers"][0]["server_owned"], true);
    assert!(snapshot["server_providers"][0]["account_id"].is_null());
    assert!(snapshot["client_executors"].as_array().unwrap().is_empty());
    assert!(!snapshot.to_string().contains("private-account-fixture"));
    let response = call(
        &app,
        "GET",
        "/api/control/state",
        Some(&owner),
        serde_json::Value::Null,
        None,
    )
    .await;
    let snapshot = value(response).await;
    assert_eq!(
        snapshot["server_providers"][0]["account_id"],
        "private-account-fixture"
    );
    assert_eq!(snapshot["client_executors"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn personal_profile_and_link_routes_are_available_without_access_manage() {
    let (_directory, _access, app, owner) = fixture();
    let guest = paired(&app, &owner, &[P::StudioView]).await;
    let token = guest.token.as_deref().unwrap();
    let response = call(
        &app,
        "PUT",
        "/api/access/profile",
        Some(token),
        serde_json::json!({"name":"Participant"}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let profile: Session = serde_json::from_value(value(response).await).unwrap();
    assert_eq!(profile.user.id, guest.session.user.id);
    assert_eq!(profile.user.name, "Participant");
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/access/devices",
            Some(token),
            serde_json::json!({}),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/access/invitations",
            Some(token),
            serde_json::json!({"permissions":["studio_view"]}),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/access/link-invitations",
            Some(token),
            serde_json::json!({"permissions":["access_manage"]}),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // The endpoint accepts no caller-selected owner identity.
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/access/link-invitations",
            Some(token),
            serde_json::json!({"permissions":["studio_view"], "user_id":"someone-else"}),
            None
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let response = call(
        &app,
        "POST",
        "/api/access/link-invitations",
        Some(token),
        serde_json::json!({"permissions":["studio_view"]}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let code = value(response).await;
    let response = call(
        &app,
        "POST",
        "/api/access/pair",
        None,
        serde_json::json!({"code":code["code"],"name":"My tablet"}),
        None,
    )
    .await;
    let tablet: PairResponse = serde_json::from_value(value(response).await).unwrap();
    assert_eq!(tablet.session.user, profile.user);
    let mine = value(
        call(
            &app,
            "GET",
            "/api/access/my-devices",
            Some(token),
            serde_json::json!({}),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(mine["devices"].as_array().unwrap().len(), 2);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/access/devices/{}", tablet.session.device.id),
            Some(token),
            serde_json::json!({}),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/access/session",
            tablet.token.as_deref(),
            serde_json::json!({}),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let other = paired(&app, &owner, &[P::StudioView]).await;
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/access/devices/{}", other.session.device.id),
            Some(token),
            serde_json::json!({}),
            None
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    for (method, path) in [
        ("GET", "/api/access/my-devices"),
        ("PUT", "/api/access/profile"),
        ("POST", "/api/access/link-invitations"),
    ] {
        assert_eq!(
            call(&app, method, path, None, serde_json::json!({}), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
