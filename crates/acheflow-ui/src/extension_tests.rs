//! Actual shared UI: disabled cards, retained settings and workspace isolation.
use crate::{control_client::ControlClient, project::Workspace};
use acheflow_media_api::control::{ControlSnapshot, ExtensionStatus};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct Harness {
    client: Cell<Option<ControlClient>>,
}
fn app() -> Element {
    let harness = consume_context::<Rc<Harness>>();
    let project = use_signal(Workspace::default);
    crate::live_client::use_live_client(project);
    let client = ControlClient {
        project,
        state: use_signal(|| {
            Some(ControlSnapshot {
                workspace_id: Some("other-workspace".into()),
                extensions: vec![ExtensionStatus {
                    id: "twitch".into(),
                    name: "Twitch".into(),
                    account: Some("other-account".into()),
                    settings: vec![
                        acheflow_extension_api::integration::Setting {
                            id: "client_id".into(),
                            label: "Application ID".into(),
                            ..Default::default()
                        },
                        acheflow_extension_api::integration::Setting {
                            id: "access_token".into(),
                            label: "Access token".into(),
                            secret: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            })
        }),
        error: use_signal(|| None),
        extension_errors: use_signal(std::collections::BTreeMap::new),
        busy: use_signal(|| false),
    };
    use_context_provider(|| client);
    harness.client.set(Some(client));
    rsx! {crate::interaction::ExtensionsPage {}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..30 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("extension UI did not settle");
}
fn toggle(doc: &mut DioxusDocument) {
    let id = doc
        .inner
        .borrow()
        .query_selector("button[role='switch']")
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
async fn secret_inputs_are_local_masked_and_cleared_when_the_workspace_changes() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    toggle(&mut doc);
    assert!(exists(&doc, "input[type='password'][disabled]"));
    let mut snapshot = client.state.read().clone().unwrap();
    snapshot.workspace_id = Some(client.project.read().id.clone());
    snapshot.extensions[0].configured_secrets = vec!["access_token".into()];
    client.state.set(Some(snapshot));
    settle(&mut doc);
    assert!(exists(&doc, "input[type='password']:not([disabled])"));
    let input = doc
        .inner
        .borrow()
        .query_selector("input[type='password']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("fixture-token-not-real".into()));
    settle(&mut doc);
    assert!(
        !serde_json::to_string(&*client.project.read())
            .unwrap()
            .contains("fixture-token-not-real")
    );
    assert!(
        !client.project.read().extensions["twitch"]
            .config
            .contains_key("access_token")
    );
    let first = client.project.read().clone();
    client.project.set(Workspace::default());
    settle(&mut doc);
    assert!(!exists(&doc, "input[type='password']"));
    client.project.set(first);
    settle(&mut doc);
    // The field remounts empty; saved metadata never fills a credential value.
    assert!(exists(&doc, "input[type='password'][value='']"));
}
#[tokio::test]
async fn extension_command_errors_stay_in_the_owning_card_including_when_collapsed() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let client = harness.client.get().unwrap();
    client.command(acheflow_media_api::control::ControlAction::ExtensionEvent {
        id: "twitch".into(),
        action_id: "connect".into(),
        value: String::new(),
        config: None,
    });
    settle(&mut doc);
    assert!(exists(&doc, ".extension-card .field-error"));
    assert!(
        client.error.read().is_none(),
        "extension errors must not appear in the shared notice"
    );
    assert!(!exists(&doc, ".extension-settings"));
    toggle(&mut doc);
    assert!(
        !exists(&doc, ".extension-card .field-error"),
        "retrying clears the previous error"
    );
    client.command(acheflow_media_api::control::ControlAction::ExtensionEvent {
        id: "twitch".into(),
        action_id: "connect".into(),
        value: String::new(),
        config: None,
    });
    settle(&mut doc);
    assert!(exists(&doc, ".extension-card .field-error"));
    client.command(acheflow_media_api::control::ControlAction::ExtensionEvent {
        id: "sample".into(),
        action_id: "increment".into(),
        value: String::new(),
        config: None,
    });
    settle(&mut doc);
    assert!(
        exists(&doc, ".extension-card .field-error"),
        "another extension must not clear this card's error"
    );
    assert!(client.error.read().is_none());
}
#[tokio::test]
async fn disabled_cards_hide_settings_and_toggling_or_switching_keeps_workspace_drafts() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    assert!(!exists(&doc, ".extension-settings"));
    toggle(&mut doc);
    assert!(client.project.read().extensions["twitch"].enabled);
    assert!(exists(&doc, ".extension-card #extension-twitch input"));
    let input = doc
        .inner
        .borrow()
        .query_selector("#extension-twitch input")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("firstpublicapp".into()));
    settle(&mut doc);
    assert_eq!(
        client.project.read().extensions["twitch"].config["client_id"],
        "firstpublicapp"
    );
    for character in "1234".chars() {
        doc.handle_ui_event(UiEvent::KeyDown(BlitzKeyEvent {
            key: Key::Character(character.to_string()),
            code: Code::Digit1,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: Some(character.to_string().into()),
        }));
    }
    settle(&mut doc);
    assert_eq!(
        client.project.read().extensions["twitch"].config["client_id"],
        "firstpublicapp1234"
    );
    toggle(&mut doc);
    assert!(!exists(&doc, ".extension-settings"));
    toggle(&mut doc);
    let first = client.project.read().clone();
    assert_eq!(
        first.extensions["twitch"].config["client_id"],
        "firstpublicapp1234"
    );
    client.project.set(Workspace::default());
    settle(&mut doc);
    assert!(!exists(&doc, ".extension-settings"));
    client.project.set(first.clone());
    settle(&mut doc);
    assert!(exists(&doc, ".extension-card #extension-twitch input"));
    assert_eq!(
        client.project.read().extensions["twitch"].config["client_id"],
        "firstpublicapp1234"
    );

    let mut snapshot = client.state.read().clone().unwrap();
    snapshot.workspace_id = Some(first.id);
    snapshot.panels = vec![acheflow_extension_api::ExtensionPanel {
        extension_id: "twitch".into(),
        title: "Twitch controls".into(),
        widgets: vec![acheflow_extension_api::PanelWidget::Label {
            text: "Workspace panel".into(),
        }],
    }];
    client.state.set(Some(snapshot));
    settle(&mut doc);
    assert!(exists(
        &doc,
        ".extension-card #extension-twitch .extension-panel"
    ));
    assert!(!exists(&doc, ".extension-card .panel"));
    client
        .project
        .write()
        .extensions
        .get_mut("twitch")
        .unwrap()
        .enabled = false;
    settle(&mut doc);
    assert!(!exists(&doc, ".extension-panel"));
}
