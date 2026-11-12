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
    crate::client_authorization::provide_accounts(project);
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
    local_client_fixture(false)
}
fn startup_client_app() -> Element {
    local_client_fixture(true)
}
fn local_client_fixture(enabled: bool) -> Element {
    // Local panel storage is provided by the client root, including in offline tests.
    let harness = consume_context::<Rc<Harness>>();
    let project = use_signal(move || Workspace {
        server_url: "http://127.0.0.1:9".into(),
        client_extensions: if enabled {
            ["twitch", "vklive"]
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        crate::project::ExtensionSettings {
                            enabled: true,
                            ..Default::default()
                        },
                    )
                })
                .collect()
        } else {
            Default::default()
        },
        ..Default::default()
    });
    crate::client_authorization::provide_accounts(project);
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
async fn preenabled_client_panels_keep_sessions_at_startup_and_after_server_changes() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(startup_client_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    for server in ["http://127.0.0.1:9", "http://127.0.0.1:10"] {
        client.project.write().server_url = server.into();
        settle(&mut doc);
        for provider in ["twitch", "vklive"] {
            let card = format!("#client-extension-{provider}");
            client
                .project
                .write()
                .client_extensions
                .get_mut(provider)
                .unwrap()
                .config
                .insert("client_id".into(), "own-public-id".into());
            settle(&mut doc);
            assert!(
                exists(
                    &doc,
                    &format!("{card} .local-extension-action.primary:not([disabled])")
                ),
                "{provider} at {server}: a configured panel must retain its session and enable Connect"
            );
        }
    }
}

#[tokio::test]
async fn application_setup_renders_guest_links_addresses_and_local_failures_in_each_card() {
    for provider in ["twitch", "youtube"] {
        let harness = Rc::new(Harness::default());
        let mut doc = DioxusDocument::new(
            VirtualDom::new(local_client_app).with_root_context(harness.clone()),
            DocumentConfig::default(),
        );
        doc.initial_build();
        settle(&mut doc);
        let mut client = harness.client.get().unwrap();
        client
            .project
            .write()
            .client_extensions
            .entry(provider.into())
            .or_default()
            .enabled = true;
        settle(&mut doc);
        let card = format!("#client-extension-{provider}");
        let disclosure = doc
            .inner
            .borrow()
            .query_selector_all(&format!("{card} .extension-disclosure-toggle"))
            .unwrap()[2];
        doc.inner.borrow_mut().set_focus_to(disclosure);
        press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
        assert!(exists(&doc, &format!("{card} a[href^='https://']")));
        let address = doc
            .inner
            .borrow()
            .query_selector(&format!("{card} .extension-copy-field input[readonly]"))
            .unwrap();
        if provider == "youtube" {
            assert!(
                address.is_none(),
                "no fabricated fixed address before loopback allocation"
            );
        } else {
            let address = address.unwrap();
            let document = doc.inner.borrow();
            let attrs = document
                .get_node(address)
                .unwrap()
                .element_data()
                .unwrap()
                .attrs();
            assert!(attrs.iter().any(|a| a.name.local.to_string() == "value"
                && a.value.as_str()
                    == format!("https://app.acheflow.com/fwd/{provider}/oauth/callback")));
            drop(document);
        }
        open_application_credentials(&mut doc, provider);
        assert_eq!(
            exists(
                &doc,
                &format!(
                    "{card} .client-authorization .extension-disclosure-content input[type='password']"
                )
            ),
            provider == "youtube"
        );
        if provider != "youtube" {
            let copy = doc
                .inner
                .borrow()
                .query_selector(&format!("{card} .extension-copy-field button"))
                .unwrap()
                .unwrap();
            doc.inner.borrow_mut().set_focus_to(copy);
            press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
            assert!(
                exists(
                    &doc,
                    &format!("{card} .extension-copy-field [role='alert']")
                ),
                "missing clipboard reports an inline error"
            );
        }
        assert!(client.error.read().is_none());
        let connect = doc
            .inner
            .borrow()
            .query_selector(&format!("{card} .local-extension-action.primary"))
            .unwrap()
            .unwrap();
        doc.inner.borrow_mut().set_focus_to(connect);
        press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
        assert!(exists(
            &doc,
            &format!("{card} .local-extension-action.primary[disabled]")
        ));
        assert!(
            !exists(
                &doc,
                &format!("{card} .client-authorization [role='status']")
            ),
            "missing input cannot open a browser even via the keyboard"
        );
        assert!(client.project.read().extensions.is_empty());
    }
}

#[tokio::test]
async fn youtube_connect_requires_public_id_and_local_secret_without_leaking_the_draft() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(local_client_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    client
        .project
        .write()
        .client_extensions
        .entry("youtube".into())
        .or_default()
        .enabled = true;
    settle(&mut doc);
    assert!(!exists(
        &doc,
        "#client-extension-youtube .client-authorization input"
    ));
    open_application_credentials(&mut doc, "youtube");
    let button = "#client-extension-youtube .local-extension-action.primary";
    assert!(exists(&doc, &format!("{button}[disabled]")));
    for (selector, value, ready) in [
        (
            "#client-extension-youtube .client-authorization .extension-disclosure-content input[type='text']",
            "own-client",
            false,
        ),
        (
            "#client-extension-youtube .client-authorization .extension-disclosure-content input[type='password']",
            "local-secret-draft",
            true,
        ),
    ] {
        let input = doc
            .inner
            .borrow()
            .query_selector(selector)
            .unwrap()
            .expect("required fields are visible before Connect");
        doc.inner.borrow_mut().set_focus_to(input);
        doc.handle_ui_event(UiEvent::Paste(value.into()));
        settle(&mut doc);
        assert_eq!(exists(&doc, &format!("{button}:not([disabled])")), ready);
    }
    assert!(
        !serde_json::to_string(&*client.project.read())
            .unwrap()
            .contains("local-secret-draft")
    );
    client
        .project
        .write()
        .client_extensions
        .get_mut("youtube")
        .unwrap()
        .config
        .insert("client_id".into(), "another-client".into());
    settle(&mut doc);
    assert!(
        exists(&doc, &format!("{button}[disabled]")),
        "another application cannot reuse an unsubmitted credential draft"
    );
}
fn open_application_credentials(doc: &mut DioxusDocument, provider: &str) {
    let disclosure = doc
        .inner
        .borrow()
        .query_selector_all(&format!(
            "#client-extension-{provider} .extension-disclosure-toggle"
        ))
        .unwrap()[0];
    doc.inner.borrow_mut().set_focus_to(disclosure);
    press_tab_key(doc, NamedKey::Enter, Code::Enter);
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
    open_application_credentials(&mut doc, id);
    let input = doc
        .inner
        .borrow()
        .query_selector(&format!("#client-extension-{id} input:not([readonly])"))
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
    open_application_credentials(&mut doc, id);
    assert!(exists(&doc, &format!("#client-extension-{id} input")));
    assert_eq!(
        client.project.read().client_extensions[id].config[field],
        "local-public-app"
    );
}

#[tokio::test]
async fn local_guest_panel_keeps_manual_tokens_out_of_workspace_and_server_payloads() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(local_client_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    toggle(&mut doc);
    assert!(!exists(
        &doc,
        ".client-authorization input[type='password']"
    ));
    let disclosure = doc
        .inner
        .borrow()
        .query_selector_all(".client-authorization button[aria-expanded='false']")
        .unwrap()[1];
    doc.inner.borrow_mut().set_focus_to(disclosure);
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    assert!(exists(&doc, ".client-authorization input[type='password']"));
    let panel = doc
        .inner
        .borrow()
        .query_selector(".client-authorization")
        .unwrap()
        .unwrap();
    let text = doc.inner.borrow().get_node(panel).unwrap().text_content();
    assert!(text.contains("Connect with Twitch"));
    assert!(text.contains("Use tokens"));
    let input = doc
        .inner
        .borrow()
        .query_selector(".client-authorization input[type='password']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(input);
    doc.handle_ui_event(UiEvent::Paste("fake-local-token-for-test".into()));
    settle(&mut doc);
    let disclosure = doc
        .inner
        .borrow()
        .query_selector(".client-authorization button[aria-expanded='true']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(disclosure);
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    assert!(!exists(
        &doc,
        ".client-authorization input[type='password']"
    ));
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    let restored = doc
        .inner
        .borrow()
        .query_selector(".client-authorization input[type='password']")
        .unwrap()
        .unwrap();
    assert_eq!(
        doc.inner
            .borrow()
            .get_node(restored)
            .unwrap()
            .element_data()
            .unwrap()
            .attrs()
            .iter()
            .find(|attribute| attribute.name.local.to_string() == "value")
            .map(|attribute| attribute.value.as_ref()),
        Some("fake-local-token-for-test")
    );
    let mut client = harness.client.get().unwrap();
    let workspace = client.project.read().clone();
    assert!(
        !serde_json::to_string(&workspace)
            .unwrap()
            .contains("fake-local-token-for-test")
    );
    assert!(
        !serde_json::to_string(&workspace.server_configuration())
            .unwrap()
            .contains("fake-local-token-for-test")
    );
    client.project.set(Workspace {
        server_url: "http://127.0.0.1:9".into(),
        ..Default::default()
    });
    settle(&mut doc);
    assert!(!exists(
        &doc,
        ".client-authorization input[type='password']"
    ));
    client.project.set(workspace);
    settle(&mut doc);
    let disclosure = doc
        .inner
        .borrow()
        .query_selector_all(".client-authorization button[aria-expanded='false']")
        .unwrap()[1];
    doc.inner.borrow_mut().set_focus_to(disclosure);
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    let input = doc
        .inner
        .borrow()
        .query_selector(".client-authorization input[type='password']")
        .unwrap()
        .unwrap();
    assert_ne!(
        doc.inner
            .borrow()
            .get_node(input)
            .unwrap()
            .element_data()
            .unwrap()
            .attrs()
            .iter()
            .find(|attribute| attribute.name.local.to_string() == "value")
            .map(|attribute| attribute.value.as_ref()),
        Some("fake-local-token-for-test")
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
    open_application_credentials(&mut doc, local);
    let input = doc
        .inner
        .borrow()
        .query_selector(&format!("#client-extension-{local} input:not([readonly])"))
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

fn masked_credential_app() -> Element {
    let mut value = use_signal(String::new);
    const CSS: &str = include_str!("dashboard.css");
    rsx! {style{"{CSS}"}
        input {r#type:"password", value:value(), oninput:move|e|value.set(e.value())}
        input {r#type:"text", value:"ordinary-public-id"}
    }
}

#[tokio::test]
async fn password_editor_preserves_input_but_only_shapes_asterisks() {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(masked_credential_app),
        DocumentConfig {
            font_ctx: Some(crate::font_context()),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    let id = doc
        .inner
        .borrow()
        .query_selector("input[type='password']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
    let value = "Az09_-*/éЖ🙂";
    doc.handle_ui_event(UiEvent::Paste(value.into()));
    settle(&mut doc);
    {
        let document = doc.inner.borrow();
        let editor = &document
            .get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .text_input_data()
            .unwrap()
            .editor;
        assert_eq!(
            editor.text(),
            value,
            "masking must never alter the submitted secret"
        );
        let layout = editor.try_layout().unwrap();
        let mut glyphs = vec![];
        for line in layout.lines() {
            for run in line.runs() {
                assert!(
                    run.font().data.as_ref()
                        == include_bytes!("../assets/SecretMask.ttf").as_slice(),
                    "password rendering must select the mask-only font"
                );
                for cluster in run.clusters() {
                    glyphs.extend(cluster.glyphs().map(|g| g.id));
                }
            }
        }
        assert_eq!(
            glyphs,
            vec![1; value.chars().count()],
            "every visible character must use the single asterisk glyph"
        );
    }
    {
        let document = doc.inner.borrow();
        let public = document
            .query_selector("input[type='text']")
            .unwrap()
            .unwrap();
        let editor = &document
            .get_node(public)
            .unwrap()
            .element_data()
            .unwrap()
            .text_input_data()
            .unwrap()
            .editor;
        assert_eq!(editor.text(), "ordinary-public-id");
        for line in editor.try_layout().unwrap().lines() {
            for run in line.runs() {
                assert!(
                    run.font().data.as_ref()
                        != include_bytes!("../assets/SecretMask.ttf").as_slice(),
                    "public identifiers must remain readable"
                );
            }
        }
    }
    doc.inner
        .borrow_mut()
        .with_text_input(id, |mut editor| editor.select_all());
    doc.handle_ui_event(UiEvent::Paste("replacement-secret".into()));
    settle(&mut doc);
    assert_eq!(
        doc.inner
            .borrow()
            .get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .text_input_data()
            .unwrap()
            .editor
            .text(),
        "replacement-secret"
    );
}
#[tokio::test]
async fn compact_extension_controls_share_rows_and_wrap_without_horizontal_overflow() {
    use blitz_traits::shell::Viewport;
    for width in [1000, 320] {
        let harness = Rc::new(Harness::default());
        let mut doc = DioxusDocument::new(
            VirtualDom::new(styled_scope_app).with_root_context(harness.clone()),
            DocumentConfig {
                font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                    "../assets/DejaVuSans.woff2"
                ))),
                ..Default::default()
            },
        );
        doc.initial_build();
        doc.inner.borrow_mut().set_viewport(Viewport {
            window_size: (width, 1200),
            ..Default::default()
        });
        settle(&mut doc);
        let mut client = harness.client.get().unwrap();
        client
            .project
            .write()
            .client_extensions
            .entry("youtube".into())
            .or_default()
            .enabled = true;
        settle(&mut doc);
        let document = doc.inner.borrow();
        let controls = document
            .query_selector_all("#client-extension-youtube .extension-functions button")
            .unwrap();
        let first = document
            .get_node(controls[0])
            .unwrap()
            .absolute_position(0., 0.);
        let points: Vec<_> = controls
            .iter()
            .map(|id| document.get_node(*id).unwrap().absolute_position(0., 0.))
            .collect();
        if width == 1000 {
            assert!(points.iter().all(|point| (point.y - first.y).abs() < 1.));
        } else {
            assert!(points.iter().any(|point| point.y > first.y + 10.));
        }
        for id in controls {
            let node = document.get_node(id).unwrap();
            let right = node.absolute_position(0., 0.).x + node.final_layout().size.width;
            assert!(
                right <= width as f32 + 1.,
                "control extends past viewport: {right}"
            );
        }
        drop(document);
        open_application_credentials(&mut doc, "youtube");
        let document = doc.inner.borrow();
        for id in document.query_selector_all("#client-extension-youtube .field, #client-extension-youtube .extension-copy-field button").unwrap() {
            let node = document.get_node(id).unwrap();
            let right = node.absolute_position(0., 0.).x + node.final_layout().size.width;
            assert!(right <= width as f32 + 1., "setup control extends past viewport: {right}");
        }
    }
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

fn live_broadcast_dropdown_app() -> Element {
    let mut selected = use_signal(String::new);
    rsx! {crate::extension_panel::PanelSelect{
        label:"Live broadcast",value:selected(),options:vec!["stream-a".into(),"stream-b".into()],
        onselect:move |value|selected.set(value)
    }}
}
#[tokio::test]
async fn live_broadcast_dropdown_supports_keyboard_selection_and_collapses_after_choice() {
    let mut doc = DioxusDocument::new(
        VirtualDom::new(live_broadcast_dropdown_app),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    assert!(!exists(&doc, "[role='listbox']"));
    let toggle = doc
        .inner
        .borrow()
        .query_selector("button[aria-expanded='false']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(toggle);
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    assert!(exists(&doc, "[role='listbox']"));
    let option = doc
        .inner
        .borrow()
        .query_selector_all("[role='option']")
        .unwrap()[1];
    doc.inner.borrow_mut().set_focus_to(option);
    press_tab_key(&mut doc, NamedKey::Enter, Code::Enter);
    assert!(!exists(&doc, "[role='listbox']"));
    let toggle = doc
        .inner
        .borrow()
        .query_selector("button[aria-expanded='false']")
        .unwrap()
        .unwrap();
    assert!(
        doc.inner
            .borrow()
            .get_node(toggle)
            .unwrap()
            .text_content()
            .contains("stream-b")
    );
}

#[tokio::test]
async fn saved_read_only_permissions_offer_an_explicit_send_grant_for_capable_installations() {
    use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(mixed_app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    settle(&mut doc);
    let mut client = harness.client.get().unwrap();
    {
        let mut state = client.state.write();
        let state = state.as_mut().unwrap();
        state.workspace_id = Some(client.project.peek().id.clone());
        state.client_capabilities = vec![CapabilityGrant {
            provider: "vklive".into(),
            account_id: "account".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }];
    }
    settle(&mut doc);
    select_scope(&mut doc, "server");
    assert!(exists(
        &doc,
        ".allow-chat-send[aria-label='Allow sending to vklive / channel']"
    ));
    client.state.write().as_mut().unwrap().client_capabilities[0]
        .capabilities
        .insert(Capability::ChatSend);
    settle(&mut doc);
    assert!(!exists(&doc, ".allow-chat-send"));
}
