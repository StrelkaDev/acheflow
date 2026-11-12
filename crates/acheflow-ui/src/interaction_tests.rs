//! The idle chat view stays usable for providers with read-only channels.
use crate::{control_client::ControlClient, project::Workspace};
use acheflow_media_api::control::ControlSnapshot;
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct Harness {
    running: Cell<Option<Signal<bool>>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
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
