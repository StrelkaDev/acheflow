use super::*;
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
        MouseEventButtons, PointerCoords, UiEvent,
    },
    shell::Viewport,
};
use dioxus_native::DioxusDocument;
use std::{cell::Cell, rc::Rc};

fn app() -> Element {
    let captured = use_context::<Rc<Cell<Option<Signal<Workspace>>>>>();
    let project = use_signal(|| {
        let mut workspace = Workspace::default();
        workspace.add_scene(Orientation::Horizontal);
        workspace.add_scene(Orientation::Vertical);
        workspace.migrate_spatial();
        workspace
    });
    captured.set(Some(project));
    crate::live_client::use_live_client(project);
    crate::control_client::use_control_client(project);
    rsx! { style {{include_str!("dashboard.css")}} Scenes {project,visible:false} }
}

fn studio_app() -> Element {
    let captured = use_context::<Rc<Cell<Option<Signal<Workspace>>>>>();
    let project = use_signal(|| {
        let mut workspace = Workspace::default();
        workspace.horizontal.base = Resolution {
            width: 640,
            height: 360,
        };
        workspace.add_source();
        let id = workspace.add_scene(Orientation::Horizontal);
        workspace.set_active(Orientation::Horizontal, Some(id));
        workspace.migrate_spatial();
        workspace
    });
    captured.set(Some(project));
    crate::live_client::use_live_client(project);
    crate::control_client::use_control_client(project);
    let histories = use_scene_histories(project);
    use_context_provider(|| histories);
    let mut in_studio = use_signal(|| true);
    rsx! {style {{include_str!("dashboard.css")}}
        button {id:"switch-editor",onclick:move |_|in_studio.set(!in_studio()),"Switch editor"}
        if in_studio() {Studio {project,visible:false}} else {Scenes {project,visible:false}}
    }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..40 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("scene history editor did not settle");
}

fn activate(doc: &mut DioxusDocument, selector: &str) {
    let point = {
        let inner = doc.inner.borrow();
        let id = inner.query_selector(selector).unwrap().unwrap();
        inner.get_node(id).unwrap().absolute_position(4., 4.)
    };
    let event = |pressed| BlitzPointerEvent {
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
    doc.handle_ui_event(UiEvent::PointerMove(event(false)));
    doc.handle_ui_event(UiEvent::PointerDown(event(true)));
    doc.handle_ui_event(UiEvent::PointerUp(event(false)));
    settle(doc);
}

fn press(doc: &mut DioxusDocument, selector: &str, key: Key, code: Code, modifiers: Modifiers) {
    let id = doc
        .inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(id);
    let mut event = BlitzKeyEvent {
        key,
        code,
        modifiers,
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

#[tokio::test]
async fn chat_undo_and_redo_stay_with_their_scene_after_switching() {
    let captured = Rc::new(Cell::<Option<Signal<Workspace>>>::new(None));
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app).with_root_context(captured.clone()),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (1200, 1000),
        ..Default::default()
    });
    settle(&mut doc);
    let project = captured.get().unwrap();
    let original = project.peek().clone();

    const CHAT: &str = ".hierarchy-add button:first-of-type";
    const UNDO: &str = "[aria-label='Scene history'] button:first-of-type";
    const REDO: &str = "[aria-label='Scene history'] button:nth-of-type(2)";

    activate(&mut doc, CHAT);
    let added = project.peek().clone();
    assert_eq!(added.sources.len(), 1);
    assert_eq!(added.scenes[0].spatial.as_ref().unwrap().objects.len(), 2);

    activate(&mut doc, UNDO);
    assert_eq!(*project.peek(), original);
    activate(&mut doc, ".scene-list button:nth-child(2)");
    let redo_disabled = format!("{REDO}[disabled]");
    assert!(
        doc.inner
            .borrow()
            .query_selector(&redo_disabled)
            .unwrap()
            .is_some()
    );

    activate(&mut doc, ".scene-list button:first-child");
    activate(&mut doc, REDO);
    assert_eq!(*project.peek(), added);

    activate(&mut doc, ".scene-list button:nth-child(2)");
    activate(&mut doc, CHAT);
    let added_to_second = project.peek().clone();
    assert_eq!(added_to_second.sources.len(), 2);
    activate(&mut doc, ".scene-list button:first-child");
    activate(&mut doc, UNDO);
    let current = project.peek();
    assert_eq!(current.sources, added_to_second.sources[1..]);
    assert_eq!(current.scenes[0], original.scenes[0]);
    assert_eq!(current.scenes[1], added_to_second.scenes[1]);
}

#[tokio::test]
async fn studio_draft_layer_can_be_undone_in_scenes() {
    let captured = Rc::new(Cell::<Option<Signal<Workspace>>>::new(None));
    let mut doc = DioxusDocument::new(
        VirtualDom::new(studio_app).with_root_context(captured.clone()),
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner.borrow_mut().set_viewport(Viewport {
        window_size: (900, 1600),
        ..Default::default()
    });
    settle(&mut doc);

    let project = captured.get().unwrap();
    press(
        &mut doc,
        ".studio-edit-draft",
        Key::Named(NamedKey::Enter),
        Code::Enter,
        Modifiers::empty(),
    );
    press(
        &mut doc,
        ".studio-draft-inspector .padded.row button",
        Key::Named(NamedKey::Enter),
        Code::Enter,
        Modifiers::empty(),
    );
    assert_eq!(project.peek().scenes[0].layers.len(), 1);
    assert!(
        doc.inner
            .borrow()
            .query_selector(".studio-draft-inspector .layer-select[aria-pressed=true]")
            .unwrap()
            .is_some()
    );

    press(
        &mut doc,
        ".studio-draft-preview .scene-canvas",
        Key::Character("z".into()),
        Code::KeyZ,
        Modifiers::CONTROL,
    );
    assert!(project.peek().scenes[0].layers.is_empty());
    press(
        &mut doc,
        ".studio-draft-preview .scene-canvas",
        Key::Character("z".into()),
        Code::KeyZ,
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    assert_eq!(project.peek().scenes[0].layers.len(), 1);

    activate(&mut doc, "#switch-editor");
    activate(
        &mut doc,
        "[aria-label='Scene history'] button:first-of-type",
    );
    assert!(project.peek().scenes[0].layers.is_empty());
}
