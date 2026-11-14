use super::*;
use acheflow_media_api::{
    access::{Device, Permission as P, Session, User},
    control::ControlSnapshot,
    live::LiveStatus,
};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Harness {
    endpoint: String,
    personal: std::cell::Cell<Option<Signal<bool>>>,
}
fn installation() -> acheflow_client::catalog::Installed {
    acheflow_client::catalog::Installed {manifest:serde_json::from_value(json!({"id":"alpha","name":"Fixture","api_version":2,"module":"fixture.wasm","execution_side":"client","services":[],"capabilities":["channel","chat","secret_commit","storage"],"secret_fields":{"access_token":"/token"}})).unwrap(),module:include_bytes!("../../../../acheflow-server/tests/fixtures/domain.wasm").to_vec(),browser_auth:None}
}
fn snapshot() -> ControlSnapshot {
    ControlSnapshot {
        workspace_id: Some("workspace".into()),
        client_capabilities: vec![CapabilityGrant {
            provider: "alpha".into(),
            account_id: "verified-bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead, Capability::ChatSend].into(),
        }],
        ..Default::default()
    }
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let endpoint = harness.endpoint.clone();
    let personal = use_signal(|| true);
    harness.personal.set(Some(personal));
    let project = use_signal(move || {
        let mut w = crate::project::Workspace {
            id: "workspace".into(),
            server_url: endpoint,
            ..Default::default()
        };
        let s = w.client_extensions.entry("alpha".into()).or_default();
        s.enabled = true;
        s.config = BTreeMap::from([
            ("server_fixture".into(), "true".into()),
            ("server_authorization_fixture".into(), "true".into()),
        ]);
        w
    });
    let mut live = crate::live_client::use_live_client(project);
    use_effect(move || {
        live.status.set(Some(LiveStatus {
            workspace_id: Some("workspace".into()),
            activated: true,
            ..Default::default()
        }))
    });
    let access = crate::server_access::AccessState {
        session: use_signal(|| {
            Some(Session {
                server_id: "server".into(),
                user: User {
                    id: "user".into(),
                    name: "User".into(),
                },
                device: Device {
                    id: "device".into(),
                    user_id: "user".into(),
                    name: "Device".into(),
                    permissions: [P::StudioView, P::ExtensionsRun].into(),
                    expires_at_ms: u64::MAX,
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    let client = crate::control_client::ControlClient {
        project,
        state: use_signal(|| Some(snapshot())),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
        request_generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    let catalog = use_hook(|| {
        let mut c = ClientCatalog::default();
        c.installed.insert("alpha".into(), installation());
        Rc::new(c)
    });
    use_context_provider(|| catalog);
    let sessions: Signal<Sessions> = use_signal(move || {
        let installed = installation();
        let mut local = LocalSession::new(
            installed.manifest,
            &installed.module,
            project.peek().client_extensions["alpha"].config.clone(),
        )
        .unwrap();
        local.bind_workspace("workspace").unwrap();
        local
            .set_secret("access_token", "synthetic-token".into())
            .unwrap();
        local
            .step(
                json!({"type":"local_server_authorize"}),
                &json!({"browser":false}),
                1,
            )
            .unwrap();
        BTreeMap::from([(
            (
                "workspace".into(),
                project.peek().server_url.clone(),
                "alpha".into(),
            ),
            local,
        )])
    });
    use_context_provider(|| sessions);
    rsx! {if !personal(){ClientExecutionPanel{id:"alpha"}}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("collection UI did not settle");
}
#[tokio::test]
async fn personal_mode_does_not_collect_and_studio_mode_connects_automatically() {
    let count = Arc::new(AtomicUsize::new(0));
    let requests = count.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/api/control/state",
                axum::routing::get(|| async { axum::Json(snapshot()) }),
            )
            .route(
                "/api/live/status",
                axum::routing::get(|| async {
                    axum::Json(LiveStatus {
                        workspace_id: Some("workspace".into()),
                        activated: true,
                        ..Default::default()
                    })
                }),
            )
            .route(
                "/api/control/executor",
                axum::routing::post(move || {
                    let requests = requests.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });
    let harness = Rc::new(Harness {
        endpoint,
        personal: Default::default(),
    });
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        settle(&mut doc);
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "Committing credentials must not register a collector"
    );
    harness.personal.get().unwrap().set(false);
    settle(&mut doc);
    let deadline = Instant::now() + Duration::from_secs(3);
    while count.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "Explicit local start did not register a collector: {}",
            doc.inner.borrow().root_element().outer_html()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
        settle(&mut doc);
    }
    harness.personal.get().unwrap().set(true);
    settle(&mut doc);
    let stopped = count.load(Ordering::SeqCst);
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        settle(&mut doc);
    }
    assert_eq!(count.load(Ordering::SeqCst), stopped);
    task.abort();
}
