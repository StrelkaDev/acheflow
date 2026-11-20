use super::*;
use acheflow_media_api::access::{Device, Permission as P, Session, User};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
use serde_json::json;
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct Harness {
    endpoint: String,
    text: Cell<Option<Signal<String>>>,
    visible: Cell<Option<Signal<bool>>>,
}
fn app() -> Element {
    let h = use_context::<Rc<Harness>>();
    let endpoint = h.endpoint.clone();
    let project = use_signal(move || crate::project::Workspace {
        id: "workspace".into(),
        server_url: endpoint,
        ..Default::default()
    });
    let access = server_access::AccessState {
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
                    permissions: [P::ChatSend].into(),
                    expires_at_ms: u64::MAX,
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    let client = ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
        request_generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    let catalog = use_hook(|| {
        let mut catalog = ClientCatalog::default();
        let manifest=serde_json::from_value(json!({"id":"alpha","name":"Alpha","api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":["channel","http","delivery","secret_commit","storage"],"services":["https://provider.example"],"secret_fields":{"access_token":"/access_token"},"subscriptions":["chat.send"],"supported_actions":["chat.send"]})).unwrap();
        catalog.installed.insert(
            "alpha".into(),
            acheflow_client::catalog::Installed {
                manifest,
                module: include_bytes!("../../../../acheflow-server/tests/fixtures/domain.wasm")
                    .to_vec(),
                browser_auth: None,
            },
        );
        Rc::new(catalog)
    });
    use_context_provider(|| catalog.clone());
    let sessions = use_signal(|| {
        let installed = &catalog.installed["alpha"];
        let mut local = acheflow_client::local::LocalSession::new(
            installed.manifest.clone(),
            &installed.module,
            std::collections::BTreeMap::from([(
                "server_authorization_fixture".into(),
                "true".into(),
            )]),
        )
        .unwrap();
        local.bind_workspace("workspace").unwrap();
        local
            .set_secret("access_token", "device-only".into())
            .unwrap();
        local
            .step(
                json!({"type":"local_server_authorize"}),
                &json!({"browser":false}),
                100,
            )
            .unwrap();
        Sessions::from([(
            ("workspace".into(), h.endpoint.clone(), "alpha".into()),
            local,
        )])
    });
    use_context_provider(|| sessions);
    let draft = ChatDraft {
        text: use_signal(String::new),
        targets: use_signal(|| vec!["alpha:channel".into()]),
        personal_pending: use_signal(|| None),
    };
    use_context_provider(|| draft);
    h.text.set(Some(draft.text));
    let visible = use_signal(|| true);
    h.visible.set(Some(visible));
    rsx! {if visible(){Composer{channels:vec![Channel{stream_id:None,extension_id:"alpha".into(),id:"channel".into(),label:"Read-only shared feed".into(),capabilities:vec!["chat.read".into()],constraints:Default::default()}],text:draft.text}}}
}
fn settle(d: &mut DioxusDocument) {
    for _ in 0..40 {
        if !d.poll(None) {
            d.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("UI did not settle")
}
fn click(d: &mut DioxusDocument) {
    use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
    let id = d
        .inner
        .borrow()
        .query_selector(".personal-chat-compose button.primary")
        .unwrap()
        .unwrap();
    d.inner.borrow_mut().set_focus_to(id);
    let mut e = BlitzKeyEvent {
        key: Key::Named(NamedKey::Enter),
        code: Code::Enter,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    };
    d.handle_ui_event(UiEvent::KeyDown(e.clone()));
    e.state = KeyState::Released;
    d.handle_ui_event(UiEvent::KeyUp(e));
    settle(d);
}
#[tokio::test]
async fn device_sender_checks_server_permission_without_uploading_credentials_or_falling_back() {
    let requests = Arc::new(Mutex::new(Vec::<AuthorizeDirectAction>::new()));
    let captured = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/api/access/actions/authorize",
            axum::routing::post(
                move |axum::Json(request): axum::Json<AuthorizeDirectAction>| {
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().push(request);
                        axum::http::StatusCode::FORBIDDEN
                    }
                },
            ),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let h = Rc::new(Harness {
        endpoint,
        ..Default::default()
    });
    let mut d = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(h.clone()),
        DocumentConfig::default(),
    );
    d.initial_build();
    settle(&mut d);
    h.text.get().unwrap().set("Hello".into());
    settle(&mut d);
    assert!(
        d.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("verified-bot")
    );
    click(&mut d);
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests.lock().unwrap().is_empty() {
        settle(&mut d);
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for _ in 0..5 {
        settle(&mut d);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    h.visible.get().unwrap().set(false);
    settle(&mut d);
    h.visible.get().unwrap().set(true);
    settle(&mut d);
    assert_eq!(h.text.get().unwrap().peek().as_str(), "Hello");
    assert_eq!(requests.lock().unwrap().len(), 1);
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].provider, "alpha");
    assert_eq!(requests[0].channel_id, "channel");
    task.abort();
}
