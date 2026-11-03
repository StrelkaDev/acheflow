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
                    settings: vec![acheflow_extension_api::integration::Setting {
                        id: "client_id".into(),
                        label: "Application ID".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })
        }),
        error: use_signal(|| None),
        extension_errors: use_signal(std::collections::BTreeMap::new),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
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

#[tokio::test]
async fn client_waiting_card_keeps_public_settings_and_never_renders_credential_fields() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    toggle(&mut doc);
    let mut client = harness.client.get().unwrap();
    let mut snapshot = client.state.read().clone().unwrap();
    snapshot.workspace_id = Some(client.project.read().id.clone());
    snapshot.extensions[0].state = "waiting_for_client".into();
    snapshot.extensions[0]
        .settings
        .push(acheflow_extension_api::integration::Setting {
            id: "credential".into(),
            label: "Must never render".into(),
            secret: true,
            ..Default::default()
        });
    client.state.set(Some(snapshot));
    settle(&mut doc);
    let document = doc.inner.borrow();
    let count = document
        .query_selector_all("#extension-twitch input")
        .unwrap()
        .len();
    assert_eq!(
        count, 1,
        "only the public Application ID field should remain"
    );
    assert!(
        document
            .query_selector(".extension-card .badge")
            .unwrap()
            .is_some()
    );
    assert!(
        document
            .query_selector("input[type='password']")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn first_snapshot_cannot_acknowledge_remote_settings_over_an_unsynced_local_copy() {
    use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    client.state.set(None);
    client.project.write().edit_revision = 1;
    client
        .project
        .write()
        .extensions
        .entry("twitch".into())
        .or_default()
        .config
        .insert("client_id".into(), "localdraft".into());
    let snapshot = ControlSnapshot {
        workspace_id: Some(client.project.read().id.clone()),
        edit_revision: 2,
        revision: 2,
        client_capabilities: vec![CapabilityGrant {
            provider: "twitch".into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }],
        ..Default::default()
    };
    client.accept(snapshot.clone());
    settle(&mut doc);
    assert_eq!(
        client.project.read().edit_revision,
        1,
        "unreconciled settings must keep their old revision"
    );
    assert_eq!(
        client.project.read().extensions["twitch"].config["client_id"],
        "localdraft"
    );
    assert!(client.error.read().is_some());
    client.accept(snapshot);
    settle(&mut doc);
    assert_eq!(
        client.project.read().edit_revision,
        1,
        "a repeated poll cannot clear the conflict"
    );
}

#[tokio::test]
async fn resolving_settings_conflicts_is_explicit_and_does_not_send_an_apply() {
    use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    client.state.set(None);
    client.project.write().edit_revision = 1;
    client
        .project
        .write()
        .extensions
        .entry("twitch".into())
        .or_default()
        .config
        .insert("client_id".into(), "localdraft".into());
    let snapshot = ControlSnapshot {
        workspace_id: Some(client.project.read().id.clone()),
        edit_revision: 2,
        revision: 2,
        client_capabilities: vec![CapabilityGrant {
            provider: "twitch".into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }],
        ..Default::default()
    };
    client.accept(snapshot.clone());
    settle(&mut doc);
    assert!((client.sync_conflict)());
    assert_eq!(
        doc.inner
            .borrow()
            .query_selector_all(".panel-note button")
            .unwrap()
            .len(),
        2
    );
    client.resolve_sync_conflict(false);
    settle(&mut doc);
    assert!(!(client.sync_conflict)());
    assert_eq!(client.project.read().edit_revision, 2);
    assert_eq!(
        client.project.read().extensions["twitch"].config["client_id"],
        "localdraft"
    );
    assert!(client.project.read().client_capabilities.is_empty());
    assert!(
        !(client.busy)(),
        "resolving is a local choice; Apply stays separate"
    );
    client.state.set(None);
    client.project.write().edit_revision = 1;
    client.accept(snapshot.clone());
    settle(&mut doc);
    client.resolve_sync_conflict(true);
    settle(&mut doc);
    assert_eq!(
        client.project.read().client_capabilities,
        snapshot.client_capabilities
    );
    assert_eq!(
        client.project.read().extensions,
        snapshot.extension_settings
    );
    assert_eq!(client.project.read().edit_revision, 2);
    assert!(client.error.read().is_none());
    assert!(!(client.busy)());
}
