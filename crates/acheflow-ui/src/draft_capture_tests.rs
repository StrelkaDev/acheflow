use super::*;
use acheflow_media_api::{
    access::{Device, Permission, Session, User},
    live::{LiveStatus, SourcePreviewRequest},
};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
        UiEvent,
    },
    shell::Viewport,
};
use dioxus_native::DioxusDocument;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn app() -> Element {
    let initial = use_context::<Workspace>();
    let project = use_signal(|| initial);
    let access = crate::server_access::AccessState {
        session: use_signal(|| {
            Some(Session {
                user: User {
                    id: "fixture-user".into(),
                    name: "Fixture".into(),
                },
                server_id: "fixture-server".into(),
                device: Device {
                    id: "fixture-device".into(),
                    user_id: "fixture-user".into(),
                    name: "Fixture".into(),
                    permissions: [Permission::StudioView, Permission::MediaManage].into(),
                    expires_at_ms: u64::MAX,
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    crate::live_client::use_live_client(project);
    crate::control_client::use_control_client(project);
    rsx! {style {{include_str!("dashboard.css")}} Studio {project,visible:true}}
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..50 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("Studio did not settle");
}

async fn until(doc: &mut DioxusDocument, condition: impl Fn(&DioxusDocument) -> bool) {
    for _ in 0..300 {
        settle(doc);
        if condition(doc) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Studio capture request timed out");
}

fn click(doc: &mut DioxusDocument, selector: &str) {
    let point = {
        let dom = doc.inner.borrow();
        let id = dom.query_selector(selector).unwrap().unwrap();
        dom.get_node(id).unwrap().absolute_position(4., 4.)
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
async fn studio_switches_draft_capture_to_the_selected_scene() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut workspace = Workspace {
        server_url: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    for _ in 0..4 {
        workspace.add_source();
    }
    let first = workspace.add_scene(Orientation::Horizontal);
    let second = workspace.add_scene(Orientation::Horizontal);
    for (scene, source) in [(first, 1), (first, 2), (second, 1), (second, 3)] {
        workspace.add_layer(scene, source);
    }
    let owner = workspace.id.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    let router = axum::Router::new()
        .route(
            "/api/live/status",
            axum::routing::get(move || {
                let owner = owner.clone();
                async move {
                    axum::Json(LiveStatus {
                        workspace_id: Some(owner),
                        ..Default::default()
                    })
                }
            }),
        )
        .route(
            "/api/live/source-preview",
            axum::routing::post(
                move |axum::Json(request): axum::Json<SourcePreviewRequest>| {
                    let log = log.clone();
                    async move {
                        log.lock().unwrap().push(request.source_id);
                        axum::Json(())
                    }
                },
            ),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(workspace),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (1400, 1000),
        ..Default::default()
    });
    const CAPTURE: &str = ".draft-preview-controls button";
    until(&mut doc, |doc| {
        doc.inner
            .borrow()
            .query_selector(CAPTURE)
            .unwrap()
            .is_some()
    })
    .await;
    assert!(requests.lock().unwrap().is_empty());

    click(&mut doc, CAPTURE);
    let count = |id| {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|&&source| source == id)
            .count()
    };
    until(&mut doc, |_| count(1) == 1 && count(2) == 1).await;
    assert_eq!(count(3) + count(4), 0);

    click(
        &mut doc,
        ".studio-grid > section:first-of-type .field .row > button:nth-of-type(3)",
    );
    until(&mut doc, |_| count(3) >= 1).await;
    until(&mut doc, |_| count(1) >= 2 && count(3) >= 2).await;
    assert_eq!(count(2), 1);
    assert_eq!(count(4), 0);

    click(&mut doc, CAPTURE);
    let previous_requests = requests.lock().unwrap().len();
    let until_after_renewal = tokio::time::Instant::now() + Duration::from_millis(2200);
    until(&mut doc, |_| {
        tokio::time::Instant::now() >= until_after_renewal
    })
    .await;
    assert_eq!(requests.lock().unwrap().len(), previous_requests);
    server.abort();
}
