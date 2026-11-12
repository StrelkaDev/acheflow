//! The idle chat view stays usable for providers with read-only channels.
use crate::{control_client::ControlClient, project::Workspace};
use acheflow_media_api::control::ControlSnapshot;
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct Harness {
    running: Cell<Option<Signal<bool>>>,
    client: Cell<Option<ControlClient>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    use_context_provider(|| Rc::new(acheflow_client::catalog::ClientCatalog::bundled()));
    let project = use_signal(Workspace::default);
    crate::live_client::use_live_client(project);
    crate::interaction::use_chat_draft(project);
    let client = ControlClient {
        project,
        state: use_signal(move || {
            Some(ControlSnapshot {
                workspace_id: Some(project.peek().id.clone()),
                channels: vec![acheflow_extension_api::integration::Channel {
                    extension_id: "vklive".into(),
                    id: "channel".into(),
                    label: "Read-only channel".into(),
                    capabilities: vec!["chat.read".into()],
                    constraints: Default::default(),
                }],
                ..Default::default()
            })
        }),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
    };
    use_context_provider(|| client);
    harness.client.set(Some(client));
    let running = use_signal(|| false);
    harness.running.set(Some(running));
    rsx! {style{{include_str!("dashboard.css")}}div{class:"content",crate::live::StandaloneChat{running:running()}}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..30 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("chat view did not settle");
}

#[tokio::test]
async fn idle_chat_shows_read_only_channels_and_docks_when_live_starts() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let html = doc.inner.borrow().root_element().outer_html();
    assert!(html.contains("Read-only channel"));
    assert!(!html.contains("No connected channel"));
    assert!(
        doc.inner
            .borrow()
            .query_selector(".chat-compose input[disabled]")
            .unwrap()
            .is_some()
    );
    assert!(
        doc.inner
            .borrow()
            .query_selector(".chat-history")
            .unwrap()
            .is_some()
    );
    harness.running.get().unwrap().set(true);
    settle(&mut doc);
    assert!(
        doc.inner
            .borrow()
            .query_selector(".chat-docked-notice")
            .unwrap()
            .is_some()
    );
    assert!(
        doc.inner
            .borrow()
            .query_selector(".chat-history")
            .unwrap()
            .is_none()
    );
    harness.running.get().unwrap().set(false);
    settle(&mut doc);
    assert!(
        doc.inner
            .borrow()
            .query_selector(".chat-history")
            .unwrap()
            .is_some()
    );
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
fn exists(doc: &DioxusDocument, selector: &str) -> bool {
    doc.inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .is_some()
}
#[tokio::test]
async fn chat_targets_support_multiple_channels_and_never_fan_out_after_disconnect() {
    use acheflow_extension_api::{
        integration::Channel,
        interaction::{ChatEntry, ChatMessage},
    };
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    let channel = |extension: &str, label: &str, send| Channel {
        extension_id: extension.into(),
        id: "channel".into(),
        label: label.into(),
        capabilities: if send {
            vec!["chat.read".into(), "chat.send".into()]
        } else {
            vec!["chat.read".into()]
        },
        constraints: Default::default(),
    };
    {
        let mut state = client.state.write();
        let state = state.as_mut().unwrap();
        state.channels = vec![
            channel("twitch", "Twitch / example", true),
            channel("youtube", "YouTube / Example broadcast", true),
            channel("vklive", "VK Video Live / example", false),
        ];
        state.chat.entries = vec![ChatEntry {
            cursor: 1,
            message: ChatMessage {
                platform: "youtube".into(),
                channel_id: "channel".into(),
                message_id: "message".into(),
                user_id: "viewer".into(),
                display_name: "Viewer".into(),
                text: "Hello from the live broadcast".into(),
                roles: vec![],
                occurred_at_ms: 1000,
            },
            deleted: false,
            merged_into: None,
            delivery: vec![],
        }];
    }
    settle(&mut doc);
    assert!(!exists(&doc, ".panel-heading select"));
    assert!(exists(&doc, ".chat-origin svg"));
    assert!(
        doc.inner
            .borrow()
            .root_element()
            .outer_html()
            .contains("Example broadcast")
    );
    assert!(exists(&doc, ".chat-targets button[disabled]"));
    activate(&mut doc, ".chat-targets button[aria-label^='Twitch']");
    activate(&mut doc, ".chat-targets button[aria-label^='YouTube']");
    assert!(exists(
        &doc,
        ".chat-targets button[aria-label^='Twitch'][aria-pressed='true']"
    ));
    assert!(exists(
        &doc,
        ".chat-targets button[aria-label^='YouTube'][aria-pressed='true']"
    ));
    if let Ok(path) = std::env::var("ACHEFLOW_CHAT_PREVIEW") {
        let html = doc.inner.borrow().root_element().outer_html();
        std::fs::write(path,format!("<!doctype html><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><style>{}</style>{html}",include_str!("dashboard.css"))).unwrap();
    }
    client
        .state
        .write()
        .as_mut()
        .unwrap()
        .channels
        .retain(|c| c.extension_id == "vklive");
    settle(&mut doc);
    assert!(exists(&doc, ".chat-compose input[disabled]"));
    // An available read-only channel must never become a new implicit recipient.
    activate(&mut doc, ".chat-targets button:not([aria-label])");
    assert!(exists(&doc, ".chat-targets button[aria-pressed='true']"));
}
