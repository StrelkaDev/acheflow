use super::*;
use acheflow_client::server::{LocalServer, Phase, Status};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
        MouseEventButtons, PointerCoords, UiEvent,
    },
    shell::Viewport,
};
use dioxus_native::DioxusDocument;
use std::{cell::Cell, rc::Rc, sync::Arc};

#[derive(Default)]
struct Harness {
    state: Cell<Option<ServerState>>,
    catalog: Cell<Option<Signal<crate::workspaces::Catalog>>>,
    project: Cell<Option<Signal<Workspace>>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let state = ServerState {
        available: use_signal(|| Some(false)),
        info: use_signal(|| None),
        error: use_signal(String::new),
        busy: use_signal(|| false),
        operation: use_signal(|| 0),
        process: use_signal(Default::default),
        external: use_signal(|| None),
        logs: use_signal(Vec::new),
    };
    use_context_provider(|| state);
    use_context_provider(|| Some(Arc::new(LocalServer::new().unwrap())));
    let project = use_signal(|| Workspace {
        server_url: "https://remote.example".into(),
        ..Default::default()
    });
    let catalog = use_signal(crate::workspaces::Catalog::default);
    let storage_status = use_signal(|| "Saved locally".into());
    let effects = use_signal(|| false);
    harness.state.set(Some(state));
    harness.project.set(Some(project));
    harness.catalog.set(Some(catalog));
    rsx! {style {{include_str!("../dashboard.css")}}div {class:"content",crate::dashboard::GeneralSettings {effects,project,catalog,storage_status}}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("server settings did not settle");
}
fn exists(doc: &DioxusDocument, selector: &str) -> bool {
    doc.inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .is_some()
}
fn press(doc: &mut DioxusDocument, key: Key, code: Code) {
    let mut event = BlitzKeyEvent {
        key,
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
fn focus(doc: &mut DioxusDocument, selector: &str) {
    let id = doc
        .inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
}

#[tokio::test]
async fn settings_logs_open_with_keyboard_and_switch_between_client_and_server() {
    crate::diagnostics::record("error", "fixture", "Fixture log line: HTTP/2 unavailable");
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../../assets/DejaVuSans.woff2"
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
    focus(&mut doc, "#settings-logs");
    press(&mut doc, Key::Named(NamedKey::Enter), Code::Enter);
    for _ in 0..5 {
        tokio::task::yield_now().await;
        settle(&mut doc);
    }
    assert!(exists(&doc, "#settings-logs[aria-pressed='true']"));
    assert!(exists(
        &doc,
        "#diagnostic-log[aria-label='Client log entries']"
    ));
    let text = doc
        .inner
        .borrow()
        .query_selector("#diagnostic-log")
        .unwrap()
        .unwrap();
    assert!(
        doc.inner
            .borrow()
            .get_node(text)
            .unwrap()
            .text_content()
            .contains("Fixture log line")
    );
    if let Ok(path) = std::env::var("ACHEFLOW_LOGS_PREVIEW") {
        std::fs::write(path, doc.inner.borrow().root_element().outer_html()).unwrap();
    }
    focus(&mut doc, "#logs-pause");
    press(&mut doc, Key::Character(" ".into()), Code::Space);
    assert!(exists(&doc, "#logs-pause[aria-pressed='true']"));
    click(&mut doc, "#logs-server");
    assert!(exists(
        &doc,
        "#diagnostic-log[aria-label='Server log entries']"
    ));
    assert!(exists(&doc, ".startup-logs"));
    let text = doc
        .inner
        .borrow()
        .query_selector("#diagnostic-log")
        .unwrap()
        .unwrap();
    assert!(
        !doc.inner
            .borrow()
            .get_node(text)
            .unwrap()
            .text_content()
            .contains("Fixture log line")
    );
    click(&mut doc, "#settings-general");
    assert!(exists(&doc, "#settings-general[aria-pressed='true']"));
    assert!(!exists(&doc, "#diagnostic-log"));
    assert!(exists(&doc, "#local-server-start"));
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
#[test]
fn network_access_is_opt_in_and_restores_from_preferences() {
    let old = r#"{"start_with_client":true,"keep_running_on_exit":false,"http_port":21295,"rtmp_port":1935}"#;
    let preferences: ServerPreferences = serde_json::from_str(old).unwrap();
    assert_eq!(
        serde_json::to_value(preferences).unwrap()["allow_network"],
        false
    );
    let network = r#"{"allow_network":true}"#;
    let preferences: ServerPreferences = serde_json::from_str(network).unwrap();
    assert_eq!(
        serde_json::to_value(preferences).unwrap()["allow_network"],
        true
    );
}

#[tokio::test]
async fn server_controls_follow_ownership_and_preferences_work_with_keyboard() {
    let harness = Rc::new(Harness::default());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../../assets/DejaVuSans.woff2"
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
    assert!(exists(&doc, "#local-server-start:not([disabled])"));
    assert!(exists(&doc, "#local-server-stop[disabled]"));
    click(&mut doc, "#server-allow-network");
    assert_eq!(
        serde_json::to_value(harness.catalog.get().unwrap().read().server_preferences()).unwrap()["allow_network"],
        true
    );
    press(&mut doc, Key::Character(" ".into()), Code::Space);
    assert_eq!(
        serde_json::to_value(harness.catalog.get().unwrap().read().server_preferences()).unwrap()["allow_network"],
        false
    );
    click(&mut doc, "#server-auto-start");
    assert!(
        harness
            .catalog
            .get()
            .unwrap()
            .read()
            .server_preferences()
            .start_with_client
    );
    press(&mut doc, Key::Character(" ".into()), Code::Space);
    assert!(
        !harness
            .catalog
            .get()
            .unwrap()
            .read()
            .server_preferences()
            .start_with_client
    );
    press(&mut doc, Key::Named(NamedKey::Tab), Code::Tab);
    press(&mut doc, Key::Character(" ".into()), Code::Space);
    assert!(
        harness
            .catalog
            .get()
            .unwrap()
            .read()
            .server_preferences()
            .keep_running_on_exit
    );
    let mut state = harness.state.get().unwrap();
    doc.vdom.in_scope(ScopeId::APP, || {
        state.process.set(Status {
            phase: Phase::Ready,
            pid: Some(42),
            ..Default::default()
        })
    });
    settle(&mut doc);
    assert!(exists(&doc, "#local-server-start[disabled]"));
    assert!(exists(&doc, "#local-server-stop:not([disabled])"));
    assert!(exists(&doc, "[aria-label='Local HTTP port'][disabled]"));
    focus(&mut doc, "#use-local-server");
    press(&mut doc, Key::Named(NamedKey::Enter), Code::Enter);
    assert_eq!(
        harness.project.get().unwrap().read().server_url,
        "http://127.0.0.1:21295"
    );
    doc.vdom.in_scope(ScopeId::APP, || {
        state.process.set(Status {
            phase: Phase::Starting,
            pid: Some(42),
            ..Default::default()
        });
        state.busy.set(true);
    });
    settle(&mut doc);
    assert!(exists(&doc, "#local-server-stop:not([disabled])"));
    assert!(exists(&doc, "#local-server-restart[disabled]"));
    assert!(exists(&doc, "#server-allow-network[disabled]"));
    doc.vdom.in_scope(ScopeId::APP, || {
        state.process.set(Status::default());
        state.busy.set(false);
        state.external.set(Some(ServerInfo {
            protocol_version: 1,
            version: "0.1.3".into(),
            process_id: 50,
            managed_instance: None,
            desktop_attached: false,
            http_address: "127.0.0.1:21295".parse().unwrap(),
            rtmp_address: "127.0.0.1:1935".parse().unwrap(),
        }));
    });
    settle(&mut doc);
    assert!(exists(&doc, "#local-server-start[disabled]"));
    assert!(exists(&doc, "#local-server-stop[disabled]"));
    assert!(exists(&doc, "#use-local-server:not([disabled])"));
}
#[test]
fn automatic_connection_does_not_replace_remote_or_unrelated_local_targets() {
    for url in [
        "https://remote.example",
        "http://127.0.0.1:22222",
        "http://localhost:21295/proxy",
        "https://localhost:21295",
    ] {
        let workspace = Workspace {
            server_url: url.into(),
            ..Default::default()
        };
        assert!(!local_target(&workspace, 21295), "{url}");
    }
    for url in [
        "http://localhost:21295",
        "http://127.0.0.1:21295",
        "http://[::1]:21295",
    ] {
        assert!(local_target(
            &Workspace {
                server_url: url.into(),
                ..Default::default()
            },
            21295
        ));
    }
}

#[test]
fn discovered_server_on_a_custom_port_connects_only_the_unchanged_local_workspace() {
    let mut workspace = Workspace {
        server_url: String::new(),
        ..Default::default()
    };
    let identity = (workspace.id.clone(), workspace.server_url.clone());
    assert_eq!(
        local_endpoint_if_current(&workspace, &identity, 22345).as_deref(),
        Some("http://127.0.0.1:22345")
    );
    workspace.server_url = "https://remote.example".into();
    assert!(local_endpoint_if_current(&workspace, &identity, 22345).is_none());
    workspace.server_url.clear();
    workspace.id.push_str("-switched");
    assert!(local_endpoint_if_current(&workspace, &identity, 22345).is_none());
}
