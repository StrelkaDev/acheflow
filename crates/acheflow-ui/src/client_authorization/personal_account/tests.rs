use super::*;
use acheflow_media_api::access::{Device, Permission as P, Session, User};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
struct Harness {
    endpoint: String,
    sessions: Cell<Option<Signal<Sessions>>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let endpoint = harness.endpoint.clone();
    let project = use_signal(move || {
        let mut project = crate::project::Workspace {
            id: "workspace".into(),
            server_url: endpoint,
            ..Default::default()
        };
        project
            .client_extensions
            .entry("vklive".into())
            .or_default()
            .enabled = true;
        project
    });
    let access = server_access::AccessState {
        session: use_signal(|| {
            Some(Session {
                server_id: "server".into(),
                user: User {
                    id: "viewer-user".into(),
                    name: "Participant".into(),
                },
                device: Device {
                    id: "phone".into(),
                    user_id: "viewer-user".into(),
                    name: "Phone".into(),
                    permissions: [P::StudioView].into(),
                    expires_at_ms: u64::MAX,
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    let client = crate::control_client::ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
        request_generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    let sessions = use_signal(Sessions::new);
    harness.sessions.set(Some(sessions));
    use_context_provider(|| sessions);
    rsx! {PersonalAccountPanel{id:"vklive"}super::super::server_account::ServerAccount{id:"vklive"}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("personal account UI did not settle");
}
fn activate(doc: &mut DioxusDocument, selector: &str) {
    use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
    let id = doc
        .inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
    let mut event = BlitzKeyEvent {
        key: Key::Named(NamedKey::Enter),
        code: Code::Enter,
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
#[tokio::test]
async fn viewer_can_manage_a_personal_account_without_administrative_access_or_a_second_chat_channel()
 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/api/access/personal-accounts/{workspace}",
            axum::routing::get(|| async {
                axum::Json(PersonalAccounts {
                    send_epoch: String::new(),
                    server_time_ms: 0,
                    accounts: vec![PersonalAccount {
                        provider: "vklive".into(),
                        account_id: "my-saved-account".into(),
                        version: "version".into(),
                    }],
                })
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let harness = Rc::new(Harness {
        endpoint: endpoint.clone(),
        sessions: Cell::new(None),
    });
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        settle(&mut doc);
        if doc
            .inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("my-saved-account")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "saved personal account did not load"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        doc.inner
            .borrow()
            .query_selector("#server-account-review")
            .unwrap()
            .is_none()
    );
    assert!(
        doc.inner
            .borrow()
            .query_selector("#personal-account-review[disabled]")
            .unwrap()
            .is_some()
    );
    assert!(
        doc.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("Disconnect my account")
    );
    let catalog = acheflow_client::catalog::ClientCatalog::bundled();
    let installed = &catalog.installed["vklive"];
    let mut local = LocalSession::new(
        installed.manifest.clone(),
        &installed.module,
        BTreeMap::new(),
    )
    .unwrap();
    local.bind_workspace("workspace").unwrap();
    local
        .set_secret("access_token", "synthetic-secret-never-render".into())
        .unwrap();
    local
        .step(
            json!({"type":"local_server_authorize"}),
            &json!({"browser":false}),
            1,
        )
        .unwrap();
    local.step(json!({"type":"local_http","id":"auth:validate","status":200,"payload":{"data":{"user":{"id":123}}}}),&json!({"browser":false}),2).unwrap();
    harness
        .sessions
        .get()
        .unwrap()
        .write()
        .insert(("workspace".into(), endpoint, "vklive".into()), local);
    settle(&mut doc);
    assert!(
        doc.inner
            .borrow()
            .query_selector("#personal-account-review[disabled]")
            .unwrap()
            .is_none()
    );
    activate(&mut doc, "#personal-account-review");
    assert!(
        doc.inner
            .borrow()
            .query_selector("#personal-account-submit[disabled]")
            .unwrap()
            .is_some()
    );
    assert!(
        !doc.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("synthetic-secret-never-render")
    );
    activate(&mut doc, "#personal-account-trust");
    assert!(
        doc.inner
            .borrow()
            .query_selector("#personal-account-submit[disabled]")
            .unwrap()
            .is_none()
    );
    assert!(
        doc.inner
            .borrow()
            .query_selector("input")
            .unwrap()
            .is_none(),
        "Personal authorization does not select or start a chat channel"
    );
    server.abort();
}
