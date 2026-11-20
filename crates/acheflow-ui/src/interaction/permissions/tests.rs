use super::*;
use acheflow_extension_api::{
    client_execution::{Connection, ExecutorStatus},
    integration::Channel,
};
use acheflow_media_api::{
    access::{Device, Permission as P, Session, User},
    control::{ControlRequest, ControlSnapshot},
};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
        MouseEventButtons, PointerCoords, UiEvent,
    },
    shell::Viewport,
};
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
    client: Cell<Option<ControlClient>>,
}

fn snapshot() -> ControlSnapshot {
    let providers = ["youtube", "twitch", "vklive"];
    ControlSnapshot {
        workspace_id: Some("workspace".into()),
        revision: 1,
        client_capabilities: providers
            .map(|provider| CapabilityGrant {
                provider: provider.into(),
                account_id: "sender".into(),
                channel_id: "channel".into(),
                capabilities: [Capability::ChatRead, Capability::AudienceRead].into(),
            })
            .into(),
        channels: providers
            .map(|provider| Channel {
                extension_id: provider.into(),
                id: "channel".into(),
                stream_id: Some("broadcast-a".into()),
                label: "Studio channel".into(),
                capabilities: vec!["chat.read".into(), "audience.read".into()],
                constraints: Default::default(),
            })
            .into(),
        client_executors: providers
            .map(|provider| ExecutorStatus {
                client_id: format!("collector-{provider}"),
                generation: 1,
                expires_at_ms: u64::MAX,
                connections: vec![Connection {
                    channel_label: None,
                    stream_id: Some("broadcast-a".into()),
                    id: "connection-a".into(),
                    provider: provider.into(),
                    account_id: "sender".into(),
                    channel_id: "channel".into(),
                    capabilities: [Capability::ChatRead, Capability::AudienceRead].into(),
                }],
            })
            .into(),
        ..Default::default()
    }
}

fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let endpoint = harness.endpoint.clone();
    let project = use_signal(move || crate::project::Workspace {
        id: "workspace".into(),
        server_url: endpoint,
        client_capabilities: snapshot().client_capabilities,
        ..Default::default()
    });
    let access = crate::server_access::AccessState {
        session: use_signal(|| {
            Some(Session {
                server_id: "server".into(),
                user: User {
                    id: "operator".into(),
                    name: "Operator".into(),
                },
                device: Device {
                    id: "device".into(),
                    user_id: "operator".into(),
                    name: "Device".into(),
                    permissions: [P::ExtensionsConfigure].into(),
                    expires_at_ms: u64::MAX,
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    let client = ControlClient {
        project,
        state: use_signal(|| Some(snapshot())),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
        request_generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    harness.client.set(Some(client));
    let catalog = use_hook(|| {
        let mut catalog = ClientCatalog::default();
        for (id, name) in [
            ("youtube", "YouTube"),
            ("twitch", "Twitch"),
            ("vklive", "VK Video Live"),
        ] {
            catalog.installed.insert(id.into(), acheflow_client::catalog::Installed {
                manifest: serde_json::from_value(json!({
                    "id": id, "name": name, "api_version": 2, "module": "fixture.wasm",
                    "execution_side": "client", "capabilities": ["channel", "delivery"],
                    "services": [], "subscriptions": ["chat.send"], "supported_actions": ["chat.send"]
                })).unwrap(),
                module: Vec::new(),
                browser_auth: None,
            });
        }
        Rc::new(catalog)
    });
    use_context_provider(|| catalog);
    rsx! {style {{include_str!("../../dashboard.css")}}
        main {class:"content",style:"padding:24px;max-width:900px",WorkspacePermissions {}}
    }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("permission settings did not settle");
}

fn document(harness: Rc<Harness>) -> DioxusDocument {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../../../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (1000, 1000),
        ..Default::default()
    });
    settle(&mut doc);
    doc
}

fn count(doc: &DioxusDocument, selector: &str) -> usize {
    doc.inner
        .borrow()
        .query_selector_all(selector)
        .unwrap()
        .len()
}

fn activate(doc: &mut DioxusDocument, selector: &str, space: bool) {
    let id = doc
        .inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
    let mut event = BlitzKeyEvent {
        key: if space {
            Key::Character(" ".into())
        } else {
            Key::Named(NamedKey::Enter)
        },
        code: if space { Code::Space } else { Code::Enter },
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    };
    doc.handle_ui_event(UiEvent::KeyDown(event.clone()));
    event.state = KeyState::Released;
    doc.handle_ui_event(UiEvent::KeyUp(event));
    settle(doc);
}

fn click(doc: &mut DioxusDocument, selector: &str) {
    let point = {
        let document = doc.inner.borrow();
        let id = document.query_selector(selector).unwrap().unwrap();
        document.get_node(id).unwrap().absolute_position(8., 8.)
    };
    let pointer = |pressed| BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: point.x,
            page_y: point.y,
            screen_x: point.x,
            screen_y: point.y,
            client_x: point.x,
            client_y: point.y,
        },
        button: MouseEventButton::Main,
        buttons: if pressed {
            MouseEventButtons::Primary
        } else {
            MouseEventButtons::None
        },
        mods: Modifiers::empty(),
        details: Default::default(),
        element: Default::default(),
        active_pointers: Default::default(),
    };
    doc.handle_ui_event(UiEvent::PointerMove(pointer(false)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(true)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(false)));
    settle(doc);
}

#[tokio::test]
async fn new_broadcast_reuses_each_provider_account_channel_card() {
    let harness = Rc::new(Harness::default());
    let mut doc = document(harness.clone());
    if let Ok(path) = std::env::var("ACHEFLOW_PERMISSION_PREVIEW") {
        let html = doc.inner.borrow().root_element().outer_html();
        std::fs::write(path, format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"></head><body>{html}</body></html>")).unwrap();
    }
    assert_eq!(count(&doc, ".permission-card"), 3);
    assert_eq!(count(&doc, ".permission-toggle[aria-pressed='true']"), 6);
    click(&mut doc, ".permission-card .permission-details");
    assert!(
        doc.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("broadcast-a")
    );

    let mut client = harness.client.get().unwrap();
    client.state.with_mut(|state| {
        let state = state.as_mut().unwrap();
        state.channels[0].stream_id = Some("broadcast-b".into());
        state.client_executors[0].generation = 2;
        state.client_executors[0].connections[0].id = "connection-b".into();
    });
    settle(&mut doc);
    assert_eq!(count(&doc, ".permission-card"), 3);
    assert_eq!(count(&doc, ".permission-toggle[aria-pressed='true']"), 6);
    let html = doc.inner.borrow().root_element().outer_html();
    assert!(html.contains("broadcast-b"));
    assert!(!html.contains("broadcast-a</dd>"));

    // The same disclosure remains usable by keyboard after mouse interaction.
    activate(&mut doc, ".permission-card .permission-details", true);
    assert_eq!(count(&doc, ".permission-identity"), 0);
    activate(&mut doc, ".permission-card .permission-details", false);
    assert_eq!(count(&doc, ".permission-identity"), 1);

    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (390, 1000),
        ..Default::default()
    });
    settle(&mut doc);
    let document = doc.inner.borrow();
    for id in document
        .query_selector_all(".workspace-capabilities button")
        .unwrap()
    {
        let node = document.get_node(id).unwrap();
        assert!(
            node.absolute_position(0., 0.).x + node.final_layout().size.width <= 390.,
            "Permission controls must fit a narrow viewport"
        );
    }
}

#[tokio::test]
async fn permission_buttons_send_policy_and_revocation_survives_reconnection() {
    let requests = Arc::new(Mutex::new(Vec::<ControlRequest>::new()));
    let server_snapshot = Arc::new(Mutex::new(snapshot()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let captured = requests.clone();
    let state = server_snapshot.clone();
    let task = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/api/control/commands",
            axum::routing::post(move |axum::Json(request): axum::Json<ControlRequest>| {
                let captured = captured.clone();
                let state = state.clone();
                async move {
                    let mut state = state.lock().unwrap();
                    assert_eq!(request.workspace_id, "workspace");
                    assert_eq!(request.expected_revision, state.revision);
                    let ControlAction::SetClientCapabilities { grants } = &request.action else {
                        panic!("permission control sent an unrelated action");
                    };
                    state.client_capabilities = grants.clone();
                    state.revision += 1;
                    captured.lock().unwrap().push(request);
                    axum::Json(state.clone())
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let harness = Rc::new(Harness {
        endpoint,
        ..Default::default()
    });
    let mut doc = document(harness.clone());
    activate(&mut doc, ".allow-chat-send", false);
    wait_for_command(&mut doc, &harness, &requests, 1).await;
    {
        let requests = requests.lock().unwrap();
        let ControlAction::SetClientCapabilities { grants } = &requests[0].action else {
            panic!("wrong action")
        };
        assert!(grants[0].capabilities.contains(&Capability::ChatSend));
        assert_eq!(&grants[1..], &snapshot().client_capabilities[1..]);
    }
    assert_eq!(count(&doc, ".permission-toggle[aria-pressed='true']"), 7);

    // Revoke through Space on the same card after its mouse disclosure was used.
    click(&mut doc, ".permission-card .permission-details");
    activate(&mut doc, ".permission-card .permission-revoke", true);
    wait_for_command(&mut doc, &harness, &requests, 2).await;
    {
        let requests = requests.lock().unwrap();
        let ControlAction::SetClientCapabilities { grants } = &requests[1].action else {
            panic!("wrong action")
        };
        assert_eq!(grants.len(), 3);
        assert_eq!(grants[0].provider, "youtube");
        assert_eq!(grants[0].account_id, "sender");
        assert_eq!(grants[0].channel_id, "channel");
        assert!(grants[0].capabilities.is_empty());
        assert_eq!(&grants[1..], &snapshot().client_capabilities[1..]);
    }
    assert_eq!(count(&doc, ".permission-revoked"), 1);
    assert_eq!(
        count(
            &doc,
            ".permission-card:first-child .permission-toggle[aria-pressed='true']"
        ),
        0
    );
    assert!(
        doc.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("Access revoked")
    );

    let mut client = harness.client.get().unwrap();
    client.state.with_mut(|state| {
        let state = state.as_mut().unwrap();
        state.channels[0].stream_id = Some("broadcast-after-revocation".into());
        state.client_executors[0].generation += 1;
        state.client_executors[0].connections[0].id = "reconnected".into();
    });
    settle(&mut doc);
    assert_eq!(count(&doc, ".permission-card"), 3);
    assert_eq!(count(&doc, ".permission-revoked"), 1);
    assert_eq!(
        count(
            &doc,
            ".permission-card:first-child .permission-toggle[aria-pressed='true']"
        ),
        0
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "a new broadcast must not rewrite saved consent"
    );
    task.abort();
}

async fn wait_for_command(
    doc: &mut DioxusDocument,
    harness: &Harness,
    requests: &Mutex<Vec<ControlRequest>>,
    expected: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        settle(doc);
        let client = harness.client.get().unwrap();
        if requests.lock().unwrap().len() == expected && !*client.busy.peek() {
            assert!(client.error.peek().is_none(), "{:?}", client.error.peek());
            return;
        }
        assert!(
            Instant::now() < deadline,
            "permission request did not finish: {:?}",
            client.error.peek()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
