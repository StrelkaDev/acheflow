use super::*;
use acheflow_media_api::{
    access::{Device, Permission, Session},
    control::{ControlAction, ControlSnapshot},
    server_workspace::{WorkspaceState, WorkspaceSummary},
};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
struct Harness {
    endpoint: String,
    sync: Cell<Option<WorkspaceSync>>,
    live: Cell<Option<crate::live_client::LiveClient>>,
    source_row: Cell<Option<ScopeId>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let project = use_signal(|| Workspace {
        id: "fixture-local".into(),
        server_url: harness.endpoint.clone(),
        ..Default::default()
    });
    let catalog = use_signal(|| {
        let mut catalog = Catalog::default();
        catalog.update_current(project.peek().clone());
        catalog
    });
    let access = crate::server_access::AccessState {
        session: use_signal(|| {
            Some(Session {
                user: acheflow_media_api::access::User {
                    id: "fixture-user".into(),
                    name: "Fixture".into(),
                },
                server_id: "fixture-server".into(),
                device: Device {
                    user_id: "fixture-user".into(),
                    id: "fixture-device".into(),
                    permissions: Permission::ALL.into(),
                    expires_at_ms: u64::MAX,
                    name: "Fixture".into(),
                },
            })
        }),
        error: use_signal(String::new),
    };
    use_context_provider(|| access);
    harness
        .live
        .set(Some(crate::live_client::use_live_client(project)));
    let control = crate::control_client::use_control_client(project);
    let sync = provide(project, catalog, control);
    harness.sync.set(Some(sync));
    rsx! {style {{include_str!("../dashboard.css")}} Notice {}
        div {class:"fixture-sources",
            for source in &project.read().sources {
                SourceRow {key:"{source.id}", project, source_id:source.id}
            }
        }
    }
}

#[component]
fn SourceRow(project: Signal<Workspace>, source_id: u32) -> Element {
    if source_id == 1 {
        use_context::<Rc<Harness>>()
            .source_row
            .set(Some(dioxus_core::current_scope_id()));
    }
    rsx! {crate::source_settings::RemoveSource {project,source_id}}
}

fn document(endpoint: &str) -> (DioxusDocument, Rc<Harness>) {
    let harness = Rc::new(Harness {
        endpoint: endpoint.into(),
        sync: Cell::new(None),
        live: Cell::new(None),
        source_row: Cell::new(None),
    });
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(harness.clone()),
        DocumentConfig::default(),
    );
    doc.initial_build();
    (doc, harness)
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("workspace sync did not settle");
}
async fn until(doc: &mut DioxusDocument, condition: impl Fn() -> bool) {
    for _ in 0..350 {
        settle(doc);
        if condition() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("workspace synchronization timed out");
}
fn state(revision: u64) -> WorkspaceState {
    let project = Workspace {
        id: "fixture-local".into(),
        edit_revision: revision,
        ..Default::default()
    };
    WorkspaceState {
        server_id: "fixture-server".into(),
        revision,
        summary: Some(WorkspaceSummary {
            id: project.id.clone(),
            name: project.name.clone(),
        }),
        workspace: Some(project),
        view: None,
    }
}

async fn camera_document() -> (DioxusDocument, Rc<Harness>, tokio::task::JoinHandle<()>) {
    use acheflow_media_api::{live::LiveStatus, visual::SourceKind};

    let mut initial = state(1);
    let workspace = initial.workspace.as_mut().unwrap();
    workspace.sources.clear();
    workspace.add_source();
    workspace.add_source();
    workspace.sources[0].kind = SourceKind::Camera {
        device_id: "fixture-camera".into(),
        mode: None,
        mirrored: false,
    };
    let server_state = Arc::new(RwLock::new(initial));
    let read = server_state.clone();
    let app = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(move || {
                let state = read.read().unwrap().clone();
                async move { axum::Json(state) }
            }),
        )
        .route(
            "/api/live/status",
            axum::routing::get(|| async {
                axum::Json(LiveStatus {
                    workspace_id: Some("fixture-local".into()),
                    ..Default::default()
                })
            }),
        )
        .route(
            "/api/live/sources",
            axum::routing::post(
                move |axum::Json(request): axum::Json<SourceSettingsRequest>| {
                    let mut state = server_state.write().unwrap();
                    assert_eq!(request.expected_revision, state.revision);
                    let revision = state.revision + 1;
                    let workspace = state.workspace.as_mut().unwrap();
                    assert!(request.edit.apply(workspace, request.source_id).unwrap());
                    workspace.edit_revision = revision;
                    state.revision = revision;
                    std::future::ready(axum::Json(SourceSettingsAck { revision }))
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut doc, harness) = document(&endpoint);
    let live = harness.live.get().unwrap();
    let sync = harness.sync.get().unwrap();
    until(&mut doc, || live.owns() && (sync.ready)()).await;
    sync.resolve(true);
    settle(&mut doc);
    assert!(sync.saved_source(1).is_some());
    (doc, harness, server)
}

#[tokio::test]
async fn camera_mirror_and_mode_apply_when_changed_together() {
    use acheflow_media_api::visual::{CameraMode, SourceKind};

    let (mut doc, harness, server) = camera_document().await;
    let live = harness.live.get().unwrap();
    let sync = harness.sync.get().unwrap();
    let mode = CameraMode {
        width: 3840,
        height: 2160,
        duration_value: 1,
        duration_scale: 60,
    };
    doc.vdom
        .runtime()
        .in_scope(harness.source_row.get().unwrap(), || {
            crate::source_settings::change_camera_settings(
                sync.project,
                1,
                "fixture-camera",
                |_, mirrored| *mirrored = true,
            );
            crate::source_settings::change_camera_settings(
                sync.project,
                1,
                "fixture-camera",
                |selected, _| *selected = Some(mode),
            );
        });
    assert!((live.scoped_pending)());
    until(&mut doc, || !(live.scoped_pending)()).await;
    assert!(live.error.peek().is_none());
    assert!(matches!(
        sync.saved_source(1).unwrap().kind,
        SourceKind::Camera { mode: Some(saved), mirrored: true, .. } if saved == mode
    ));
    assert_eq!(sync.project.peek().edit_revision, 2);
    server.abort();
}

#[tokio::test]
async fn removing_a_saved_camera_reenables_remove_for_remaining_sources() {
    let (mut doc, harness, server) = camera_document().await;
    let live = harness.live.get().unwrap();
    let sync = harness.sync.get().unwrap();
    doc.vdom
        .runtime()
        .in_scope(harness.source_row.get().unwrap(), || {
            crate::source_settings::edit_source(sync.project, 1, SourceEdit::Remove);
        });
    assert!((live.scoped_pending)());
    until(&mut doc, || live.project.peek().sources.len() == 1).await;
    assert_eq!(live.project.peek().sources[0].id, 2);
    until(&mut doc, || !(live.scoped_pending)()).await;
    assert!(
        doc.inner
            .borrow()
            .query_selector(".fixture-sources button:enabled")
            .unwrap()
            .is_some()
    );
    assert!(live.error.peek().is_none());
    server.abort();
}
#[tokio::test]
async fn two_real_ui_clients_follow_server_edits_and_present_conflict_resolution() {
    let shared = Arc::new(RwLock::new(state(1)));
    let read = shared.clone();
    let app = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(move || {
                let response = read.read().unwrap().clone();
                async move { axum::Json(response) }
            }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async { axum::Json(ControlSnapshot::default()) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut a, ha) = document(&endpoint);
    let (mut b, hb) = document(&endpoint);
    until(&mut a, || (ha.sync.get().unwrap().ready)()).await;
    until(&mut b, || (hb.sync.get().unwrap().ready)()).await;
    let mut sa = ha.sync.get().unwrap();
    let sb = hb.sync.get().unwrap();
    sa.project.write().name = "Local unsent title".into();
    {
        let mut next = shared.write().unwrap();
        next.revision = 2;
        let project = next.workspace.as_mut().unwrap();
        project.edit_revision = 2;
        project.horizontal.fps = 25;
    }
    until(&mut a, || sa.project.peek().edit_revision == 2).await;
    until(&mut b, || sb.project.peek().edit_revision == 2).await;
    assert_eq!(sa.project.peek().name, "Local unsent title");
    assert_eq!(sa.project.peek().horizontal.fps, 25);
    assert_eq!(sb.project.peek().horizontal.fps, 25);
    {
        let mut next = shared.write().unwrap();
        next.revision = 3;
        let project = next.workspace.as_mut().unwrap();
        project.edit_revision = 3;
        project.name = "Other client title".into();
    }
    until(&mut a, || (sa.control.sync_conflict)()).await;
    assert_eq!(sa.project.peek().edit_revision, 2);
    assert!(
        a.inner
            .borrow()
            .query_selector_all(".workspace-sync-note button")
            .unwrap()
            .len()
            >= 2
    );
    sa.resolve(false);
    settle(&mut a);
    assert_eq!(sa.project.peek().edit_revision, 3);
    assert_eq!(sa.project.peek().name, "Local unsent title");
    server.abort();
}
#[tokio::test]
async fn a_delayed_control_get_cannot_replace_a_newer_command_response() {
    let delay = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let revision = Arc::new(AtomicU64::new(1));
    let (flag, started, go, read_revision) = (
        delay.clone(),
        entered.clone(),
        release.clone(),
        revision.clone(),
    );
    let get = move || {
        let (flag, started, go, read_revision) = (
            flag.clone(),
            started.clone(),
            go.clone(),
            read_revision.clone(),
        );
        async move {
            let revision = read_revision.load(Ordering::Acquire);
            if flag.swap(false, Ordering::AcqRel) {
                started.notify_one();
                go.notified().await;
            }
            axum::Json(ControlSnapshot {
                workspace_id: Some("fixture-local".into()),
                revision,
                edit_revision: revision,
                ..Default::default()
            })
        }
    };
    let app = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(|| async { axum::Json(state(0)) }),
        )
        .route("/api/control/state", axum::routing::get(get))
        .route(
            "/api/control/commands",
            axum::routing::post(move || {
                revision.store(2, Ordering::Release);
                async {
                    axum::Json(ControlSnapshot {
                        workspace_id: Some("fixture-local".into()),
                        revision: 2,
                        edit_revision: 2,
                        ..Default::default()
                    })
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut doc, harness) = document(&endpoint);
    until(&mut doc, || {
        harness.sync.get().unwrap().control.state.peek().is_some()
    })
    .await;
    delay.store(true, Ordering::Release);
    let mut blocked = Box::pin(entered.notified());
    let mut reached = false;
    for _ in 0..700 {
        settle(&mut doc);
        if futures_util::poll!(blocked.as_mut()).is_ready() {
            reached = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(reached, "the old GET must actually be in flight");
    let mut client = harness.sync.get().unwrap().control;
    doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.command(ControlAction::SetClientCapabilities { grants: vec![] })
    });
    until(&mut doc, || {
        client
            .state
            .peek()
            .as_ref()
            .is_some_and(|s| s.revision == 2)
    })
    .await;
    release.notify_one();
    for _ in 0..30 {
        settle(&mut doc);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(client.state.peek().as_ref().unwrap().revision, 2);
    assert!(!(client.busy)());
    // A revoked/replaced session invalidates an outstanding response even at the same URL.
    let workspace = client.project.peek().clone();
    let identity = doc
        .vdom
        .runtime()
        .in_scope(ScopeId::ROOT, crate::server_access::identity);
    let old = *client.request_generation.peek();
    assert!(doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.response_current(old, &workspace, &identity)
    }));
    *client.request_generation.write() += 1;
    assert!(!doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.response_current(old, &workspace, &identity)
    }));
    server.abort();
}

#[tokio::test]
async fn switching_servers_cancels_a_delayed_workspace_response() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (started, go) = (entered.clone(), release.clone());
    let old = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(move || {
                let (started, go) = (started.clone(), go.clone());
                async move {
                    started.notify_one();
                    go.notified().await;
                    axum::Json(state(100))
                }
            }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async { axum::Json(ControlSnapshot::default()) }),
        );
    let new = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(|| async {
                let mut next = state(1);
                next.workspace.as_mut().unwrap().name = "Second server".into();
                axum::Json(next)
            }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async { axum::Json(ControlSnapshot::default()) }),
        );
    let a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first = format!("http://{}", a.local_addr().unwrap());
    let second = format!("http://{}", b.local_addr().unwrap());
    let old_server = tokio::spawn(async move { axum::serve(a, old).await.unwrap() });
    let new_server = tokio::spawn(async move { axum::serve(b, new).await.unwrap() });
    let (mut doc, harness) = document(&first);
    let mut blocked = Box::pin(entered.notified());
    let mut reached = false;
    for _ in 0..100 {
        settle(&mut doc);
        if futures_util::poll!(blocked.as_mut()).is_ready() {
            reached = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(reached);
    let mut sync = harness.sync.get().unwrap();
    sync.project.write().server_url = second.clone();
    until(&mut doc, || sync.project.peek().name == "Second server").await;
    release.notify_one();
    for _ in 0..30 {
        settle(&mut doc);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(sync.project.peek().server_url, second);
    assert_eq!(sync.project.peek().edit_revision, 1);
    assert_eq!(sync.project.peek().name, "Second server");
    old_server.abort();
    new_server.abort();
}

#[tokio::test]
async fn a_timed_out_command_releases_busy_state_and_allows_the_next_command() {
    let first = Arc::new(AtomicBool::new(true));
    let app = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(|| async { axum::Json(state(0)) }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async {
                axum::Json(ControlSnapshot {
                    workspace_id: Some("fixture-local".into()),
                    revision: 1,
                    ..Default::default()
                })
            }),
        )
        .route(
            "/api/control/commands",
            axum::routing::post(move || {
                let first = first.clone();
                async move {
                    if first.swap(false, Ordering::AcqRel) {
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    }
                    axum::Json(ControlSnapshot {
                        workspace_id: Some("fixture-local".into()),
                        revision: 2,
                        ..Default::default()
                    })
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut doc, harness) = document(&endpoint);
    until(&mut doc, || {
        harness.sync.get().unwrap().control.state.peek().is_some()
    })
    .await;
    let client = harness.sync.get().unwrap().control;
    doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.command(ControlAction::SetClientCapabilities { grants: vec![] })
    });
    let mut timed_out = false;
    for _ in 0..1300 {
        settle(&mut doc);
        if !(client.busy)() && client.error.peek().is_some() {
            timed_out = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        timed_out,
        "the production request deadline must release busy state"
    );
    doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.command(ControlAction::SetClientCapabilities { grants: vec![] })
    });
    until(&mut doc, || {
        client
            .state
            .peek()
            .as_ref()
            .is_some_and(|s| s.revision == 2)
    })
    .await;
    assert!(!(client.busy)());
    assert!(client.error.peek().is_none());
    server.abort();
}

#[tokio::test]
async fn a_late_command_response_cannot_replace_state_from_a_new_server() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (started, go) = (entered.clone(), release.clone());
    let old = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(|| async { axum::Json(state(0)) }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async {
                axum::Json(ControlSnapshot {
                    workspace_id: Some("fixture-local".into()),
                    revision: 1,
                    ..Default::default()
                })
            }),
        )
        .route(
            "/api/control/commands",
            axum::routing::post(move || {
                let (started, go) = (started.clone(), go.clone());
                async move {
                    started.notify_one();
                    go.notified().await;
                    axum::Json(ControlSnapshot {
                        workspace_id: Some("fixture-local".into()),
                        revision: 2,
                        ..Default::default()
                    })
                }
            }),
        );
    let new = axum::Router::new()
        .route(
            "/api/workspace",
            axum::routing::get(|| async { axum::Json(state(0)) }),
        )
        .route(
            "/api/control/state",
            axum::routing::get(|| async {
                axum::Json(ControlSnapshot {
                    workspace_id: Some("fixture-local".into()),
                    revision: 5,
                    ..Default::default()
                })
            }),
        );
    let a = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first = format!("http://{}", a.local_addr().unwrap());
    let second = format!("http://{}", b.local_addr().unwrap());
    let old_server = tokio::spawn(async move { axum::serve(a, old).await.unwrap() });
    let new_server = tokio::spawn(async move { axum::serve(b, new).await.unwrap() });
    let (mut doc, harness) = document(&first);
    until(&mut doc, || {
        harness.sync.get().unwrap().control.state.peek().is_some()
    })
    .await;
    let mut client = harness.sync.get().unwrap().control;
    doc.vdom.runtime().in_scope(ScopeId::ROOT, || {
        client.command(ControlAction::SetClientCapabilities { grants: vec![] })
    });
    let mut blocked = Box::pin(entered.notified());
    let mut reached = false;
    for _ in 0..100 {
        settle(&mut doc);
        if futures_util::poll!(blocked.as_mut()).is_ready() {
            reached = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(reached);
    client.project.write().server_url = second;
    until(&mut doc, || {
        client
            .state
            .peek()
            .as_ref()
            .is_some_and(|s| s.revision == 5)
    })
    .await;
    release.notify_one();
    for _ in 0..30 {
        settle(&mut doc);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(client.state.peek().as_ref().unwrap().revision, 5);
    assert!(!(client.busy)());
    old_server.abort();
    new_server.abort();
}
