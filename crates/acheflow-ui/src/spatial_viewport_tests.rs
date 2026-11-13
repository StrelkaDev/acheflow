//! Actual Blitz pointer dispatch: picking, axis drag, release and cancellation.
use crate::{
    project::Workspace,
    spatial_view::{self, EditorView},
    spatial_viewport::SpatialViewport,
};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent,
        KeyState, MouseEventButton, MouseEventButtons, PointerCoords, UiEvent,
    },
    shell::Viewport,
};
use dioxus_native::{DioxusDocument, prelude::*};
use glam::Vec2;
use std::{cell::Cell, rc::Rc};
struct Harness {
    project: Cell<Option<Signal<Workspace>>>,
    view: Cell<Option<Signal<EditorView>>>,
    preview: Cell<Option<Signal<bool>>>,
}
fn app() -> Element {
    let state = consume_context::<Rc<Harness>>();
    let project = use_signal(|| {
        let mut p = Workspace {
            server_url: "http://127.0.0.1:9".into(),
            ..Default::default()
        };
        p.add_scene(crate::project::Orientation::Horizontal);
        p.migrate_spatial();
        p
    });
    crate::live_client::use_live_client(project);
    crate::control_client::use_control_client(project);
    let view = use_signal(|| EditorView::new(1080.));
    let preview = use_signal(|| false);
    state.project.set(Some(project));
    state.view.set(Some(view));
    state.preview.set(Some(preview));
    let id = project.read().scenes[0].id;
    const CSS: &str = "body{margin:0}.spatial-stage{position:relative;width:800px;height:420px}.scene-gpu-preview{position:absolute;inset:0;width:100%;height:100%;pointer-events:none}.spatial-drag-shield{position:fixed;inset:0;z-index:10000}";
    rsx! {style{"{CSS}"}SpatialViewport{project,scene_id:id,orientation:crate::project::Orientation::Horizontal,view,camera_preview:preview()}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..60 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("3D view did not settle");
}
fn pointer(point: Vec2, down: bool) -> BlitzPointerEvent {
    BlitzPointerEvent {
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
        buttons: if down {
            MouseEventButtons::Primary
        } else {
            MouseEventButtons::None
        },
        mods: Modifiers::empty(),
        details: Default::default(),
        element: Default::default(),
        active_pointers: Default::default(),
    }
}
fn down(doc: &mut DioxusDocument, p: Vec2) {
    doc.handle_ui_event(UiEvent::PointerMove(pointer(p, false)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(p, true)));
    settle(doc);
}
fn escape(doc: &mut DioxusDocument) {
    doc.handle_ui_event(UiEvent::KeyDown(BlitzKeyEvent {
        key: Key::Named(NamedKey::Escape),
        code: Code::Escape,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    }));
    settle(doc);
}
#[tokio::test]
async fn actual_scene_view_selects_drags_cancels_and_protects_camera_preview() {
    let state = Rc::new(Harness {
        project: Cell::new(None),
        view: Cell::new(None),
        preview: Cell::new(None),
    });
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(state.clone()),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (900, 600),
        ..Default::default()
    });
    settle(&mut doc);
    let project = state.project.get().unwrap();
    let mut view = state.view.get().unwrap();
    let mut preview = state.preview.get().unwrap();
    let original = project.peek().scenes[0].spatial.clone().unwrap();
    let size = Vec2::new(800., 420.);
    let handle = spatial_view::handles(&view.peek(), &original, size)
        .into_iter()
        .find(|h| h.axis == 0)
        .unwrap();
    let start = handle.start.lerp(handle.end, 0.8);
    let end = start + Vec2::new(60., 0.);
    down(&mut doc, start);
    doc.handle_ui_event(UiEvent::PointerMove(pointer(end, true)));
    settle(&mut doc);
    assert!(
        project.peek().scenes[0].spatial.as_ref().unwrap().objects[0]
            .transform
            .position[0]
            > 0.
    );
    assert_eq!(
        project.peek().scenes[0].spatial.as_ref().unwrap().camera,
        original.camera
    );
    escape(&mut doc);
    assert_eq!(
        project.peek().scenes[0].spatial.as_ref().unwrap(),
        &original
    );
    assert!(
        doc.inner
            .borrow()
            .query_selector(".spatial-drag-shield")
            .unwrap()
            .is_none()
    );
    // Releasing outside the view ends the drag and leaves the accepted position.
    down(&mut doc, start);
    let outside = Vec2::new(start.x + 60., 500.);
    doc.handle_ui_event(UiEvent::PointerMove(pointer(outside, true)));
    settle(&mut doc);
    doc.handle_ui_event(UiEvent::PointerUp(pointer(outside, false)));
    settle(&mut doc);
    let accepted = project.peek().scenes[0].spatial.clone();
    doc.handle_ui_event(UiEvent::PointerMove(pointer(start, false)));
    settle(&mut doc);
    assert_eq!(project.peek().scenes[0].spatial, accepted);
    // Click geometry after deselection. Then camera preview rejects all manipulation.
    view.write().selected = None;
    settle(&mut doc);
    let scene = project.peek().scenes[0].spatial.clone().unwrap();
    let center = spatial_view::project(
        &view.peek().camera(),
        glam::Vec3::from_array(scene.objects[0].transform.position),
        size,
    )
    .unwrap();
    down(&mut doc, center);
    doc.handle_ui_event(UiEvent::PointerUp(pointer(center, false)));
    settle(&mut doc);
    assert_eq!(view.peek().selected, Some(1));
    let before_zoom = view.peek().distance;
    doc.handle_ui_event(UiEvent::Wheel(BlitzWheelEvent {
        delta: BlitzWheelDelta::Lines(0., 1.),
        coords: pointer(center, false).coords,
        buttons: MouseEventButtons::None,
        mods: Modifiers::empty(),
        element: Default::default(),
    }));
    settle(&mut doc);
    assert!(view.peek().distance < before_zoom, "wheel-up must zoom in");
    preview.set(true);
    settle(&mut doc);
    let saved_view = view.peek().clone();
    down(&mut doc, start);
    doc.handle_ui_event(UiEvent::PointerMove(pointer(end, true)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(end, false)));
    settle(&mut doc);
    assert_eq!(*view.peek(), saved_view);
    assert_eq!(project.peek().scenes[0].spatial, accepted);
}
