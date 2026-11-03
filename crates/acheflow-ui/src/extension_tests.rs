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
    test_app(false)
}
fn mixed_app() -> Element {
    test_app(true)
}
fn test_app(include_clients: bool) -> Element {
    let harness = consume_context::<Rc<Harness>>();
    let project = use_signal(Workspace::default);
    crate::live_client::use_live_client(project);
    let client = ControlClient {
        project,
        state: use_signal(|| {
            Some(ControlSnapshot {
                workspace_id: Some("other-workspace".into()),
                extensions: vec![ExtensionStatus {
                    id: "logic".into(),
                    name: "Logic".into(),
                    account: Some("other-account".into()),
                    settings: vec![acheflow_extension_api::integration::Setting {
                        id: "application".into(),
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
    use_context_provider(|| {
        Rc::new(if include_clients {
            acheflow_client::catalog::ClientCatalog::bundled()
        } else {
            acheflow_client::catalog::ClientCatalog::default()
        })
    });
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
        .query_selector("[role='tabpanel']:not([hidden]) button[role='switch']")
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
        id: "logic".into(),
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
        id: "logic".into(),
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
    assert!(client.project.read().extensions["logic"].enabled);
    assert!(exists(&doc, ".extension-card #extension-logic input"));
    let input = doc
        .inner
        .borrow()
        .query_selector("#extension-logic input")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("firstpublicapp".into()));
    settle(&mut doc);
    assert_eq!(
        client.project.read().extensions["logic"].config["application"],
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
        client.project.read().extensions["logic"].config["application"],
        "firstpublicapp1234"
    );
    toggle(&mut doc);
    assert!(!exists(&doc, ".extension-settings"));
    toggle(&mut doc);
    let first = client.project.read().clone();
    assert_eq!(
        first.extensions["logic"].config["application"],
        "firstpublicapp1234"
    );
    client.project.set(Workspace::default());
    settle(&mut doc);
    assert!(!exists(&doc, ".extension-settings"));
    client.project.set(first.clone());
    settle(&mut doc);
    assert!(exists(&doc, ".extension-card #extension-logic input"));
    assert_eq!(
        client.project.read().extensions["logic"].config["application"],
        "firstpublicapp1234"
    );

    let mut snapshot = client.state.read().clone().unwrap();
    snapshot.workspace_id = Some(first.id);
    snapshot.panels = vec![acheflow_extension_api::ExtensionPanel {
        extension_id: "logic".into(),
        title: "Twitch controls".into(),
        widgets: vec![acheflow_extension_api::PanelWidget::Label {
            text: "Workspace panel".into(),
        }],
    }];
    client.state.set(Some(snapshot));
    settle(&mut doc);
    assert!(exists(
        &doc,
        ".extension-card #extension-logic .extension-panel"
    ));
    assert!(!exists(&doc, ".extension-card .panel"));
    client
        .project
        .write()
        .extensions
        .get_mut("logic")
        .unwrap()
        .enabled = false;
    settle(&mut doc);
    assert!(!exists(&doc, ".extension-panel"));
}

#[tokio::test]
async fn server_logic_card_keeps_public_settings_and_never_renders_credential_fields() {
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
    snapshot.extensions[0].state = "running".into();
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
        .query_selector_all("#extension-logic input")
        .unwrap()
        .len();
    assert_eq!(
        count, 1,
        "only the public Application ID field should remain"
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
        .entry("logic".into())
        .or_default()
        .config
        .insert("application".into(), "localdraft".into());
    let snapshot = ControlSnapshot {
        workspace_id: Some(client.project.read().id.clone()),
        edit_revision: 2,
        revision: 2,
        client_capabilities: vec![CapabilityGrant {
            provider: "logic".into(),
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
        client.project.read().extensions["logic"].config["application"],
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
    client
        .project
        .write()
        .client_extensions
        .entry("client-only".into())
        .or_default()
        .config
        .insert("application".into(), "keep-local".into());
    client.state.set(None);
    client.project.write().edit_revision = 1;
    client
        .project
        .write()
        .extensions
        .entry("logic".into())
        .or_default()
        .config
        .insert("application".into(), "localdraft".into());
    let snapshot = ControlSnapshot {
        workspace_id: Some(client.project.read().id.clone()),
        edit_revision: 2,
        revision: 2,
        client_capabilities: vec![CapabilityGrant {
            provider: "logic".into(),
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
        client.project.read().extensions["logic"].config["application"],
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
    assert_eq!(
        client.project.read().client_extensions["client-only"].config["application"],
        "keep-local"
    );
    assert!(client.error.read().is_none());
    assert!(!(client.busy)());
}

fn local_client_app() -> Element {
    let harness = consume_context::<Rc<Harness>>();
    let project = use_signal(|| Workspace {
        server_url: "http://127.0.0.1:9".into(),
        ..Default::default()
    });
    crate::live_client::use_live_client(project);
    let client = ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(Default::default),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
    };
    use_context_provider(|| Rc::new(acheflow_client::catalog::ClientCatalog::bundled()));
    use_context_provider(|| client);
    harness.client.set(Some(client));
    rsx! {crate::interaction::ExtensionsPage{}}
}
#[tokio::test]
async fn client_installations_are_editable_offline_and_never_mutate_server_settings() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(local_client_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    assert!(exists(&doc, ".client-integration"));
    assert!(client.state.read().is_none());
    toggle(&mut doc);
    let catalog = acheflow_client::catalog::ClientCatalog::bundled();
    let id = catalog.installed.keys().next().unwrap();
    assert!(client.project.read().client_extensions[id].enabled);
    assert!(client.project.read().extensions.is_empty());
    assert!(!(client.busy)());
    let input = doc
        .inner
        .borrow()
        .query_selector(&format!("#client-extension-{id} input"))
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("local-public-app".into()));
    settle(&mut doc);
    let field = &catalog.installed[id].manifest.settings[0].id;
    assert_eq!(
        client.project.read().client_extensions[id].config[field],
        "local-public-app"
    );
    let first = client.project.read().clone();
    assert!(first.server_configuration().client_extensions.is_empty());
    assert!(
        !serde_json::to_string(&first.server_configuration())
            .unwrap()
            .contains("local-public-app")
    );
    client.project.set(Workspace {
        server_url: "http://127.0.0.1:9".into(),
        ..Default::default()
    });
    settle(&mut doc);
    assert!(!exists(&doc, &format!("#client-extension-{id} input")));
    client.project.set(first);
    settle(&mut doc);
    assert!(exists(&doc, &format!("#client-extension-{id} input")));
    assert_eq!(
        client.project.read().client_extensions[id].config[field],
        "local-public-app"
    );
}

fn press_tab_key(doc: &mut DioxusDocument, key: NamedKey, code: Code) {
    let mut event = BlitzKeyEvent {
        key: Key::Named(key),
        code,
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
fn select_scope(doc: &mut DioxusDocument, scope: &str) {
    let id = doc
        .inner
        .borrow()
        .query_selector(&format!("#extensions-tab-{scope}"))
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
    press_tab_key(doc, NamedKey::Enter, Code::Enter);
}
#[tokio::test]
async fn extension_scope_tabs_keep_drafts_separate_and_follow_keyboard_focus() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(mixed_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let client = harness.client.get().unwrap();
    assert!(exists(&doc, "#extensions-tab-client[aria-selected='true']"));
    assert!(exists(&doc, "#extensions-panel-client:not([hidden])"));
    assert!(exists(&doc, "#extensions-panel-server[hidden]"));
    toggle(&mut doc);
    let catalog = acheflow_client::catalog::ClientCatalog::bundled();
    let local = catalog.installed.keys().next().unwrap();
    let input = doc
        .inner
        .borrow()
        .query_selector(&format!("#client-extension-{local} input"))
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("retained-local-app".into()));
    settle(&mut doc);
    select_scope(&mut doc, "server");
    assert!(exists(
        &doc,
        "#extensions-panel-server:not([hidden]) .workspace-capabilities"
    ));
    assert!(exists(&doc, "#extensions-panel-client[hidden]"));
    toggle(&mut doc);
    assert!(client.project.read().extensions["logic"].enabled);
    select_scope(&mut doc, "client");
    assert_eq!(
        client.project.read().client_extensions[local].config
            [&catalog.installed[local].manifest.settings[0].id],
        "retained-local-app"
    );
    assert!(client.project.read().extensions["logic"].enabled);
    let client_tab = doc
        .inner
        .borrow()
        .query_selector("#extensions-tab-client")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(client_tab);
    press_tab_key(&mut doc, NamedKey::ArrowRight, Code::ArrowRight);
    assert!(exists(
        &doc,
        "#extensions-tab-server[aria-selected='true'][tabindex='0']"
    ));
    // No manual refocus: ArrowLeft must be handled by the newly focused server tab.
    press_tab_key(&mut doc, NamedKey::ArrowLeft, Code::ArrowLeft);
    assert!(exists(&doc, "#extensions-tab-client[aria-selected='true']"));
    press_tab_key(&mut doc, NamedKey::End, Code::End);
    assert!(exists(&doc, "#extensions-tab-server[aria-selected='true']"));
    press_tab_key(&mut doc, NamedKey::Home, Code::Home);
    assert!(exists(&doc, "#extensions-tab-client[aria-selected='true']"));
    assert_eq!(
        doc.inner
            .borrow()
            .query_selector_all("[role='tab'][tabindex='0']")
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn server_tab_labels_the_selected_server_and_hides_its_notices_from_local_integrations() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(mixed_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    client.project.write().server_url = "http://127.0.0.1:12341".into();
    client.error.set(Some("Selected server is offline".into()));
    settle(&mut doc);
    assert!(!exists(&doc, "#extensions-panel-client [role='alert']"));
    assert!(exists(
        &doc,
        "#extensions-panel-server[hidden] [role='alert']"
    ));
    select_scope(&mut doc, "server");
    let context = doc
        .inner
        .borrow()
        .query_selector("#extensions-panel-server .extension-scope-context")
        .unwrap()
        .unwrap();
    // Rendered text comes from the workspace's selected endpoint, not a provider installation.
    assert!(
        doc.inner
            .borrow()
            .get_node(context)
            .unwrap()
            .text_content()
            .contains("12341")
    );
    client.project.write().server_url = "http://127.0.0.1:12342".into();
    settle(&mut doc);
    let context = doc
        .inner
        .borrow()
        .query_selector("#extensions-panel-server .extension-scope-context")
        .unwrap()
        .unwrap();
    assert!(
        doc.inner
            .borrow()
            .get_node(context)
            .unwrap()
            .text_content()
            .contains("12342")
    );
}

fn styled_scope_app() -> Element {
    let content = mixed_app();
    const CSS: &str = include_str!("dashboard.css");
    rsx! {style{"{CSS}"}{content}}
}
#[tokio::test]
async fn clicking_an_inactive_scope_tab_moves_focus_before_the_next_arrow_key() {
    use blitz_traits::{
        events::{
            BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
        },
        shell::Viewport,
    };
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(styled_scope_app).with_root_context(harness),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (1000, 800),
        ..Default::default()
    });
    settle(&mut doc);
    let point = {
        let document = doc.inner.borrow();
        let id = document
            .query_selector("#extensions-tab-server")
            .unwrap()
            .unwrap();
        document.get_node(id).unwrap().absolute_position(4., 4.)
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
    settle(&mut doc);
    assert!(exists(&doc, "#extensions-tab-server[aria-selected='true']"));
    press_tab_key(&mut doc, NamedKey::ArrowLeft, Code::ArrowLeft);
    assert!(exists(&doc, "#extensions-tab-client[aria-selected='true']"));
}
