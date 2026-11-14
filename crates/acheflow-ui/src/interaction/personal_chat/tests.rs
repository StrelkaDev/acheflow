use super::*;
use acheflow_media_api::access::{Device, Permission as P, Session, User};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
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
    let draft = ChatDraft {
        text: use_signal(String::new),
        targets: use_signal(Vec::new),
        personal_mode: use_signal(|| true),
        personal_pending: use_signal(|| None),
    };
    use_context_provider(|| draft);
    h.text.set(Some(draft.text));
    let visible = use_signal(|| true);
    h.visible.set(Some(visible));
    rsx! {if visible(){Composer{channels:vec![Channel{extension_id:"alpha".into(),id:"channel".into(),label:"Read-only shared feed".into(),capabilities:vec!["chat.read".into()],constraints:Default::default()}],text:draft.text}}}
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
async fn personal_sender_uses_own_endpoint_and_preserves_uncertain_request_across_navigation() {
    let requests = Arc::new(Mutex::new(Vec::<SendPersonalChat>::new()));
    let captured = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/api/access/personal-accounts/{workspace}",
                axum::routing::get(|| async {
                    axum::Json(PersonalAccounts {
                        send_epoch: "epoch".into(),
                        server_time_ms: 1,
                        accounts: vec![PersonalAccount {
                            provider: "alpha".into(),
                            account_id: "mine".into(),
                            version: "version".into(),
                        }],
                    })
                }),
            )
            .route(
                "/api/access/personal-send/workspace/alpha/channel",
                axum::routing::post(move |axum::Json(r): axum::Json<SendPersonalChat>| {
                    let captured = captured.clone();
                    async move {
                        let mut requests = captured.lock().unwrap();
                        requests.push(r);
                        axum::Json(PersonalDelivery {
                            outcome: if requests.len() == 1 {
                                ExecutionOutcome::Unknown
                            } else {
                                ExecutionOutcome::Sent
                            },
                        })
                    }
                }),
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
    h.text.get().unwrap().set("Hello".into());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        settle(&mut d);
        if d.inner
            .borrow()
            .query_selector(".personal-chat-compose button.primary[disabled]")
            .unwrap()
            .is_none()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    click(&mut d);
    loop {
        settle(&mut d);
        if d.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("Delivery is uncertain")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    h.visible.get().unwrap().set(false);
    settle(&mut d);
    h.visible.get().unwrap().set(true);
    settle(&mut d);
    assert_eq!(requests.lock().unwrap().len(), 1);
    click(&mut d);
    loop {
        settle(&mut d);
        if h.text.get().unwrap().peek().is_empty() {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_id, requests[1].request_id);
    assert_eq!(requests[0].text, requests[1].text);
    task.abort();
}
