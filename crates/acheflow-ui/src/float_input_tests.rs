use super::FloatInput;
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{BlitzKeyEvent, KeyState, UiEvent},
    shell::Viewport,
};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};
struct Harness {
    value: Cell<Option<Signal<f32>>>,
}
fn app() -> Element {
    let state = consume_context::<Rc<Harness>>();
    let mut value = use_signal(|| 0f32);
    state.value.set(Some(value));
    rsx! {FloatInput{label:"Rotation",value:value(),change:move|v|value.set(v)}}
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..30 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("float input did not settle");
}
fn key(doc: &mut DioxusDocument, key: Key, code: Code, text: Option<&str>) {
    let mut event = BlitzKeyEvent {
        key,
        code,
        modifiers: Modifiers::empty(),
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: text.map(Into::into),
    };
    doc.handle_ui_event(UiEvent::KeyDown(event.clone()));
    event.state = KeyState::Released;
    doc.handle_ui_event(UiEvent::KeyUp(event));
}
fn text(doc: &DioxusDocument) -> String {
    let inner = doc.inner.borrow();
    let id = inner.query_selector("input").unwrap().unwrap();
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
fn replace(doc: &mut DioxusDocument, value: &str) {
    let id = doc.inner.borrow().query_selector("input").unwrap().unwrap();
    {
        let mut inner = doc.inner.borrow_mut();
        inner.set_focus_to(id);
        inner.with_text_input(id, |mut e| e.select_all());
    }
    doc.handle_ui_event(UiEvent::Paste(value.into()));
    settle(doc);
}
#[tokio::test]
async fn queued_digits_commit_together_and_invalid_or_cancelled_edits_keep_value() {
    let state = Rc::new(Harness {
        value: Cell::new(None),
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
        window_size: (600, 400),
        ..Default::default()
    });
    settle(&mut doc);
    replace(&mut doc, "");
    key(
        &mut doc,
        Key::Character("4".into()),
        Code::Digit4,
        Some("4"),
    );
    key(
        &mut doc,
        Key::Character("5".into()),
        Code::Digit5,
        Some("5"),
    );
    settle(&mut doc);
    assert_eq!(text(&doc), "45");
    assert_eq!(*state.value.get().unwrap().peek(), 0.);
    key(&mut doc, Key::Named(NamedKey::Enter), Code::Enter, None);
    settle(&mut doc);
    assert_eq!(*state.value.get().unwrap().peek(), 45.);
    replace(&mut doc, "NaN");
    key(&mut doc, Key::Named(NamedKey::Enter), Code::Enter, None);
    settle(&mut doc);
    assert_eq!(*state.value.get().unwrap().peek(), 45.);
    assert_eq!(text(&doc), "45");
    replace(&mut doc, "-2.5");
    key(&mut doc, Key::Named(NamedKey::Escape), Code::Escape, None);
    settle(&mut doc);
    assert_eq!(*state.value.get().unwrap().peek(), 45.);
    assert_eq!(text(&doc), "45");
}
