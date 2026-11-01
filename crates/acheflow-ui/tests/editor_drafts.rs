//! Exercise editor state through the same Dioxus/Blitz event path as native/WASM.
#[path = "../src/code_editor.rs"]
mod code_editor;

use blitz_dom::{Document, DocumentConfig, ScrollBehavior};
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
    MouseEventButtons, PointerCoords, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use code_editor::{CodeEditor, CodeKind};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct EditorState {
    kind: CodeKind,
    value: RefCell<String>,
    changes: RefCell<Vec<String>>,
}

fn document_config() -> DocumentConfig {
    DocumentConfig {
        font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
            "../assets/DejaVuSans.woff2"
        ))),
        ..Default::default()
    }
}

fn app() -> Element {
    let state = consume_context::<Rc<EditorState>>();
    let value = state.value.borrow().clone();
    rsx! {
        CodeEditor {
            label: "Layer CSS",
            kind: state.kind,
            value,
            change: move |text| state.changes.borrow_mut().push(text),
        }
    }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..20 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("editor did not become idle");
}

fn editor_text(doc: &DioxusDocument) -> String {
    text_at(doc, "textarea")
}

fn text_at(doc: &DioxusDocument, selector: &str) -> String {
    let inner = doc.inner.borrow();
    let id = inner.query_selector(selector).unwrap().unwrap();
    inner
        .get_node(id)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor
        .text()
        .to_string()
}

fn selection_at(doc: &DioxusDocument, selector: &str) -> std::ops::Range<usize> {
    let inner = doc.inner.borrow();
    let id = inner.query_selector(selector).unwrap().unwrap();
    inner
        .get_node(id)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor
        .raw_selection()
        .text_range()
}

fn editor(kind: CodeKind, value: &str) -> (Rc<EditorState>, DioxusDocument) {
    let state = Rc::new(EditorState {
        kind,
        value: RefCell::new(value.into()),
        changes: RefCell::new(vec![]),
    });
    let vdom = VirtualDom::new(app).with_root_context(state.clone());
    let mut doc = DioxusDocument::new(vdom, document_config());
    doc.initial_build();
    settle(&mut doc);
    (state, doc)
}

fn replace_input(doc: &mut DioxusDocument, text: &str) {
    replace_input_at(doc, "textarea", text);
}

fn replace_input_at(doc: &mut DioxusDocument, selector: &str, text: &str) {
    let textarea = doc
        .inner
        .borrow()
        .query_selector(selector)
        .unwrap()
        .unwrap();
    {
        let mut inner = doc.inner.borrow_mut();
        inner.set_focus_to(textarea);
        inner.with_text_input(textarea, |mut editor| editor.select_all());
    }
    doc.handle_ui_event(UiEvent::Paste(text.into()));
    settle(doc);
    assert_eq!(
        text_at(doc, selector),
        text,
        "input event should reach the actual editor"
    );
}

#[tokio::test]
async fn props_refresh_preserves_unapplied_css_and_wgsl_drafts() {
    for (kind, original, refreshed, pending) in [
        (
            CodeKind::Css,
            ".layer { opacity: .8; }",
            ".layer { opacity: .4; }",
            ".layer { background: url(",
        ),
        (
            CodeKind::Wgsl,
            "fn effect(uv:vec2f,color:vec4f)->vec4f{return color;}",
            "fn effect(uv:vec2f,color:vec4f)->vec4f{return vec4f(1.);}",
            "fn effect(",
        ),
    ] {
        let (state, mut doc) = editor(kind, original);
        replace_input(&mut doc, pending);

        *state.value.borrow_mut() = refreshed.into();
        doc.vdom.mark_dirty(ScopeId::APP);
        settle(&mut doc);
        assert_eq!(
            editor_text(&doc),
            pending,
            "refreshing applied props must preserve the editor's dirty draft"
        );
        assert!(
            state.changes.borrow().is_empty(),
            "invalid/uncommitted input must not be applied by a props refresh"
        );
    }
}

#[tokio::test]
async fn props_refresh_updates_a_clean_editor() {
    let (state, mut doc) = editor(CodeKind::Css, ".layer { opacity: .8; }");
    *state.value.borrow_mut() = ".layer { opacity: .4; }".into();
    doc.vdom.mark_dirty(ScopeId::APP);
    settle(&mut doc);
    assert_eq!(editor_text(&doc), *state.value.borrow());
}

fn scrolling_editor() -> Element {
    rsx! {
        style {"{include_str!(\"../src/dashboard.css\")}"}
        div {style:"width:600px;", {app()} }
    }
}

fn scrolling_document(text: &str) -> DioxusDocument {
    let state = Rc::new(EditorState {
        kind: CodeKind::Css,
        value: RefCell::new(text.into()),
        changes: RefCell::new(vec![]),
    });
    let vdom = VirtualDom::new(scrolling_editor).with_root_context(state);
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1., ColorScheme::Dark)),
            ..document_config()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    doc
}

async fn settle_scrollbars(doc: &mut DioxusDocument) {
    // Layout can reveal overflow on the other axis after reserving one rail.
    for _ in 0..3 {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        settle(doc);
    }
}

fn visible_rails(doc: &DioxusDocument) -> (bool, bool) {
    let inner = doc.inner.borrow();
    (
        inner
            .query_selector(".code-rail.horizontal")
            .unwrap()
            .is_some(),
        inner
            .query_selector(".code-rail.vertical")
            .unwrap()
            .is_some(),
    )
}

#[tokio::test]
async fn rails_only_appear_on_axes_with_overflow_and_disappear_after_shortening() {
    let long_line = format!("/* {} */", "W".repeat(240));
    let many_lines = "/* line */\n".repeat(60);
    for (text, expected) in [
        ("/* short */".to_owned(), (false, false)),
        (long_line.clone(), (true, false)),
        (many_lines.clone(), (false, true)),
        (format!("{long_line}\n{many_lines}"), (true, true)),
    ] {
        let mut doc = scrolling_document(&text);
        settle_scrollbars(&mut doc).await;
        assert_eq!(
            visible_rails(&doc),
            expected,
            "overflow axes for {expected:?}"
        );

        let scroll_id = doc
            .inner
            .borrow()
            .query_selector(".code-scroll")
            .unwrap()
            .unwrap();
        doc.inner
            .borrow_mut()
            .scroll_to(scroll_id, 10000., 10000., ScrollBehavior::Instant);

        replace_input(&mut doc, "/* short again */");
        settle_scrollbars(&mut doc).await;
        assert_eq!(visible_rails(&doc), (false, false));
        assert_eq!(
            *doc.inner
                .borrow()
                .get_node(scroll_id)
                .unwrap()
                .scroll_offset(),
            blitz_dom::util::Point { x: 0., y: 0. }
        );

        let inner = doc.inner.borrow();
        let frame = inner
            .get_node(inner.query_selector(".code-frame").unwrap().unwrap())
            .unwrap();
        let scroll = inner
            .get_node(inner.query_selector(".code-scroll").unwrap().unwrap())
            .unwrap();
        assert!(
            (frame.client_width() - scroll.client_width()).abs() < 0.5,
            "no unused vertical gutter"
        );
        assert!(
            (frame.client_height() - scroll.client_height()).abs() < 0.5,
            "no unused horizontal gutter"
        );
    }
}

fn key(doc: &mut DioxusDocument, named: NamedKey, code: Code) {
    let mut event = BlitzKeyEvent {
        key: Key::Named(named),
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

fn pointer(x: f32, y: f32, pressed: bool) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
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
    }
}

#[tokio::test]
async fn persistent_rails_scroll_by_keyboard_and_thumb_drag_without_changing_selection() {
    let text = format!(
        ".layer {{ --long: {}; }}\n{}",
        "W".repeat(240),
        ".layer { opacity: .8; }\n".repeat(60)
    );
    let mut doc = scrolling_document(&text);
    // Rail metrics sample the post-layout document on the normal timer tick.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    settle(&mut doc);
    let (scroll, horizontal, vertical, textarea) = {
        let inner = doc.inner.borrow();
        (
            inner.query_selector(".code-scroll").unwrap().unwrap(),
            inner
                .query_selector(".code-rail.horizontal")
                .unwrap()
                .expect("persistent horizontal rail"),
            inner
                .query_selector(".code-rail.vertical")
                .unwrap()
                .expect("persistent vertical rail"),
            inner.query_selector("textarea").unwrap().unwrap(),
        )
    };
    {
        let mut inner = doc.inner.borrow_mut();
        inner.set_focus_to(textarea);
        inner.with_text_input(textarea, |mut editor| editor.select_all());
    }
    let selected = selection_at(&doc, "textarea");
    assert_eq!(selected, 0..text.len());
    {
        let inner = doc.inner.borrow();
        for rail in [horizontal, vertical] {
            let node = inner.get_node(rail).unwrap();
            assert_eq!(node.attr(blitz_dom::local_name!("role")), Some("scrollbar"));
            assert!(
                node.attr(blitz_dom::local_name!("aria-valuemax"))
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
                    > 100.,
                "rail must expose its range after initial layout"
            );
            assert!(node.final_layout().size.width > 0. && node.final_layout().size.height > 0.);
        }
    }
    doc.inner.borrow_mut().set_focus_to(horizontal);
    key(&mut doc, NamedKey::End, Code::End);
    assert!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .x
            > 100.
    );
    key(&mut doc, NamedKey::Home, Code::Home);
    assert_eq!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .x,
        0.
    );
    key(&mut doc, NamedKey::ArrowRight, Code::ArrowRight);
    assert_eq!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .x,
        40.
    );
    doc.inner.borrow_mut().set_focus_to(vertical);
    key(&mut doc, NamedKey::End, Code::End);
    assert!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .y
            > 100.
    );
    key(&mut doc, NamedKey::Home, Code::Home);
    assert_eq!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .y,
        0.
    );

    for (selector, dx, dy) in [
        (".code-rail.horizontal .code-thumb", 40., 0.),
        (".code-rail.vertical .code-thumb", 0., 40.),
    ] {
        let (x, y) = {
            let inner = doc.inner.borrow();
            let thumb = inner.query_selector(selector).unwrap().unwrap();
            let node = inner.get_node(thumb).unwrap();
            let p = node.absolute_position(
                node.final_layout().size.width * 0.5,
                node.final_layout().size.height * 0.5,
            );
            (p.x, p.y)
        };
        let before = *doc.inner.borrow().get_node(scroll).unwrap().scroll_offset();
        doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, false)));
        settle(&mut doc);
        doc.handle_ui_event(UiEvent::PointerDown(pointer(x, y, true)));
        settle(&mut doc);
        doc.handle_ui_event(UiEvent::PointerMove(pointer(x + dx, y + dy, true)));
        settle(&mut doc);
        doc.handle_ui_event(UiEvent::PointerUp(pointer(x + dx, y + dy, false)));
        settle(&mut doc);
        let after = *doc.inner.borrow().get_node(scroll).unwrap().scroll_offset();
        assert!(
            if dx > 0. {
                after.x > before.x
            } else {
                after.y > before.y
            },
            "dragging {selector} must move its content axis: {before:?} → {after:?}"
        );
    }
    let (x, y) = {
        let inner = doc.inner.borrow();
        let rail = inner.get_node(vertical).unwrap();
        let point = rail.absolute_position(
            rail.final_layout().size.width * 0.5,
            rail.final_layout().size.height * 0.8,
        );
        (point.x, point.y)
    };
    let before = doc
        .inner
        .borrow()
        .get_node(scroll)
        .unwrap()
        .scroll_offset()
        .y;
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, false)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(x, y, true)));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, false)));
    settle(&mut doc);
    assert!(
        doc.inner
            .borrow()
            .get_node(scroll)
            .unwrap()
            .scroll_offset()
            .y
            > before,
        "clicking the track below the thumb scrolls downward"
    );
    assert_eq!(
        selection_at(&doc, "textarea"),
        selected,
        "rail actions preserve the editor selection"
    );
    assert_eq!(editor_text(&doc), text);
}

#[tokio::test]
async fn long_code_scrolls_in_both_axes_and_highlighting_preserves_text_and_cursor() {
    let text = format!(
        ".layer {{ --long: {}; }}\n{}",
        "W".repeat(240),
        ".layer { opacity: .8; }\n".repeat(60)
    );
    let state = Rc::new(EditorState {
        kind: CodeKind::Css,
        value: RefCell::new(text.clone()),
        changes: RefCell::new(vec![]),
    });
    let vdom = VirtualDom::new(scrolling_editor).with_root_context(state);
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1., ColorScheme::Dark)),
            ..document_config()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    assert_eq!(editor_text(&doc), text);
    let scroll = doc
        .inner
        .borrow()
        .query_selector(".code-scroll")
        .unwrap()
        .expect("code editor needs an outer scroll container");
    {
        let inner = doc.inner.borrow();
        let node = inner.get_node(scroll).unwrap();
        assert!(
            node.scroll_width() > node.client_width() + 100.,
            "long lines must overflow horizontally: {} / {}",
            node.scroll_width(),
            node.client_width()
        );
        assert!(
            node.scroll_height() > node.client_height() + 100.,
            "many lines must overflow vertically: {} / {}",
            node.scroll_height(),
            node.client_height()
        );
    }
    doc.inner
        .borrow_mut()
        .scroll_to(scroll, 100., 120., ScrollBehavior::Instant);
    let position = *doc.inner.borrow().get_node(scroll).unwrap().scroll_offset();
    assert!(
        position.x > 0. && position.y > 0.,
        "both scroll offsets must move: {position:?}"
    );
    assert_eq!(editor_text(&doc), text);

    let edited = format!("{text}/* Unicode: цвета 🦀 */\n");
    replace_input(&mut doc, &edited);
    let inner = doc.inner.borrow();
    let id = inner.query_selector("textarea").unwrap().unwrap();
    let editor = &inner
        .get_node(id)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor;
    assert_eq!(
        editor.raw_selection().text_range(),
        edited.len()..edited.len(),
        "syntax/layout rerender must keep the insertion caret"
    );
    assert!(
        inner
            .query_selector(".code-highlight span")
            .unwrap()
            .is_some(),
        "shared syntax layer should render token spans"
    );
}

struct Owners {
    selected: Cell<usize>,
    order: RefCell<Vec<usize>>,
    changes: RefCell<Vec<(usize, String)>>,
}

fn retained_editors() -> Element {
    let state = consume_context::<Rc<Owners>>();
    rsx! {
        for owner in state.order.borrow().iter().copied() {
            div {
                key: "{owner}",
                id: "owner-{owner}",
                style: if state.selected.get() == owner {"display:block"} else {"display:none"},
                CodeEditor {
                    label: "Layer CSS",
                    kind: CodeKind::Css,
                    value: format!(".layer {{ opacity: .{owner}; }}"),
                    change: {let state = state.clone(); move |text| state.changes.borrow_mut().push((owner, text))},
                }
            }
        }
    }
}

#[tokio::test]
async fn retained_owner_editors_keep_drafts_and_selection_after_switch_and_reorder() {
    let state = Rc::new(Owners {
        selected: Cell::new(1),
        order: RefCell::new(vec![1, 2]),
        changes: RefCell::new(vec![]),
    });
    let vdom = VirtualDom::new(retained_editors).with_root_context(state.clone());
    let mut doc = DioxusDocument::new(vdom, document_config());
    doc.initial_build();
    settle(&mut doc);
    replace_input_at(&mut doc, "#owner-1 textarea", ".layer { background: url(");
    let first_id = doc
        .inner
        .borrow()
        .query_selector("#owner-1 textarea")
        .unwrap()
        .unwrap();
    doc.inner
        .borrow_mut()
        .with_text_input(first_id, |mut editor| editor.select_all());
    assert_eq!(
        selection_at(&doc, "#owner-1 textarea"),
        0..".layer { background: url(".len(),
        "select_all establishes the selection"
    );

    state.selected.set(2);
    doc.vdom.mark_dirty(ScopeId::APP);
    settle(&mut doc);
    assert_eq!(
        selection_at(&doc, "#owner-1 textarea"),
        0..".layer { background: url(".len(),
        "hiding owner 1 preserves selection"
    );
    assert_eq!(
        text_at(&doc, "#owner-1 textarea"),
        ".layer { background: url(",
        "hiding owner 1 must retain its native text buffer"
    );
    replace_input_at(&mut doc, "#owner-2 textarea", ".layer { opacity:");
    assert_eq!(
        selection_at(&doc, "#owner-1 textarea"),
        0..".layer { background: url(".len(),
        "focusing and typing owner 2 preserves owner 1 selection"
    );
    assert_eq!(
        text_at(&doc, "#owner-1 textarea"),
        ".layer { background: url("
    );
    state.order.borrow_mut().reverse();
    state.selected.set(1);
    doc.vdom.mark_dirty(ScopeId::APP);
    settle(&mut doc);

    assert_eq!(
        text_at(&doc, "#owner-1 textarea"),
        ".layer { background: url("
    );
    assert_eq!(text_at(&doc, "#owner-2 textarea"), ".layer { opacity:");
    let inner = doc.inner.borrow();
    let after = inner.query_selector("#owner-1 textarea").unwrap().unwrap();
    assert_eq!(after, first_id, "reordering preserves the editor instance");
    let editor = &inner
        .get_node(after)
        .unwrap()
        .element_data()
        .unwrap()
        .text_input_data()
        .unwrap()
        .editor;
    assert_eq!(
        editor.raw_selection().text_range(),
        0..".layer { background: url(".len()
    );
    assert!(state.changes.borrow().is_empty());
}
