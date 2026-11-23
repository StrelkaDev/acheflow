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
    let catalog = use_signal(crate::workspaces::Catalog::default);
    crate::audio_monitor::provide(project, catalog);
    let separate = use_signal(|| None);
    use_context_provider(|| crate::source_settings::SeparateSourcePreview(separate));
    rsx! {style {{include_str!("dashboard.css")}}
        if let Some(source_id)=separate() {
            crate::source_settings::SeparateSourceView {project,source_id}
        } else {
            Studio {project,visible:true}
        }
    }
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

#[tokio::test]
async fn studio_source_preview_switches_between_camera_and_microphone_and_stops_on_close() {
    use acheflow_media_api::{
        audio::{AudioMeter, AudioTrackStatus},
        live::SourceStatus,
        visual::SourceKind,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut workspace = Workspace {
        server_url: format!("http://{}", listener.local_addr().unwrap()),
        ..Default::default()
    };
    workspace.add_source();
    workspace.add_source();
    workspace.sources[0].name = "Camera fixture".into();
    workspace.sources[0].kind = SourceKind::Camera {
        device_id: "fixture-camera".into(),
        mode: None,
        mirrored: false,
    };
    workspace.sources[1].name = "Microphone fixture".into();
    workspace.sources[1].kind = SourceKind::Microphone {
        device_id: "fixture-microphone".into(),
        mode: None,
    };
    let camera = workspace.sources[0].id;
    let microphone = workspace.sources[1].id;
    let owner = workspace.id.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let leases = requests.clone();
    let snapshots = requests.clone();
    let photo = {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::from_pixel(16, 9, image::Rgb([255, 0, 0])))
            .unwrap();
        bytes
    };
    let router = axum::Router::new()
        .route(
            "/api/live/status",
            axum::routing::get(move || {
                let owner = owner.clone();
                async move {
                    let status =
                        |id, width: Option<u32>, height, audio_tracks: Vec<_>| SourceStatus {
                            id,
                            connected: true,
                            received_bytes: 0,
                            bitrate_kbps: 0.,
                            frames: u64::from(width.is_some()),
                            width,
                            height,
                            fps: None,
                            audio: !audio_tracks.is_empty(),
                            audio_tracks,
                            error: None,
                        };
                    axum::Json(LiveStatus {
                        workspace_id: Some(owner),
                        sources: vec![
                            status(camera, Some(16), Some(9), vec![]),
                            status(
                                microphone,
                                None,
                                None,
                                vec![AudioTrackStatus {
                                    track_id: 0,
                                    codec: "PCM".into(),
                                    sample_rate: 48_000,
                                    channels: 2,
                                    active: true,
                                    meter: AudioMeter {
                                        peak: [0.5, 0.5],
                                        ..Default::default()
                                    },
                                    error: None,
                                }],
                            ),
                        ],
                        ..Default::default()
                    })
                }
            }),
        )
        .route(
            "/api/live/source-preview",
            axum::routing::post(
                move |axum::Json(request): axum::Json<SourcePreviewRequest>| {
                    let leases = leases.clone();
                    async move {
                        leases.lock().unwrap().push(("lease", request.source_id));
                        axum::Json(())
                    }
                },
            ),
        )
        .route(
            "/api/live/source/{id}",
            axum::routing::get(move |axum::extract::Path(id): axum::extract::Path<u32>| {
                let snapshots = snapshots.clone();
                let photo = photo.clone();
                async move {
                    snapshots.lock().unwrap().push(("video", id));
                    ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], photo)
                }
            }),
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
    settle(&mut doc);
    assert!(requests.lock().unwrap().is_empty());
    const TOGGLE: &str = ".studio-source-preview .panel-heading button";
    click(&mut doc, TOGGLE);
    assert!(requests.lock().unwrap().is_empty());
    click(&mut doc, ".studio-source-options button:first-of-type");
    let count = |kind, id| {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|&&(request_kind, source)| request_kind == kind && source == id)
            .count()
    };
    until(&mut doc, |_| {
        count("lease", camera) > 0 && count("video", camera) > 0
    })
    .await;

    click(&mut doc, ".studio-source-options button:nth-of-type(2)");
    until(&mut doc, |doc| {
        count("lease", microphone) > 0
            && doc
                .inner
                .borrow()
                .query_selector(".studio-source-track [role=meter]")
                .unwrap()
                .is_some()
    })
    .await;
    assert!(
        doc.inner
            .borrow()
            .query_selector(".studio-source-image")
            .unwrap()
            .is_none()
    );
    assert_eq!(count("video", microphone), 0);

    click(
        &mut doc,
        ".studio-source-preview .row.padded > button:not([aria-pressed])",
    );
    until(&mut doc, |doc| {
        let inner = doc.inner.borrow();
        let title = inner.query_selector(".separate-source-view h1").unwrap();
        title.is_some_and(|title| {
            inner.get_node(title).unwrap().text_content() == "Microphone fixture"
        }) && inner
            .query_selector(".separate-source-view .audio-monitor-control button")
            .unwrap()
            .is_some()
    })
    .await;
    click(&mut doc, ".separate-source-view .page-heading button");
    assert!(
        doc.inner
            .borrow()
            .query_selector(".studio-source-track")
            .unwrap()
            .is_none()
    );
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2_200);
    // Any in-flight camera fetch can finish, but no requests are scheduled after close.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let previous = requests.lock().unwrap().len();
    until(&mut doc, |_| tokio::time::Instant::now() >= deadline).await;
    assert_eq!(requests.lock().unwrap().len(), previous);
    server.abort();
}
