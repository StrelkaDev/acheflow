//! Exercise actual keyboard/pointer events, including dragging outside the rail.
use crate::slider::Slider;
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::{
    events::{
        BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, KeyState, MouseEventButton,
        MouseEventButtons, PointerCoords, UiEvent,
    },
    shell::Viewport,
};
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};
struct Harness {
    value: Cell<Option<Signal<i32>>>,
    balance: Cell<Option<Signal<i32>>>,
}
fn app() -> Element {
    let state = consume_context::<Rc<Harness>>();
    let value = use_signal(|| 100);
    state.value.set(Some(value));
    let balance = use_signal(|| 0);
    state.balance.set(Some(balance));
    const CSS: &str = ".slider-control {position:relative;width:200px;height:28px}.slider-control.vertical {width:36px;height:120px}.slider-drag-shield {position:fixed;inset:0;z-index:10000}";
    const DASHBOARD: &str = include_str!("dashboard.css");
    rsx! {style {"{DASHBOARD}"}style {"{CSS}"}
        Slider {label:"Gain",value:value(),min:0,max:200,reset:Some(100),change:move |v|{let mut value=value;value.set(v);}}
        Slider {label:"Vertical gain",value:value(),min:0,max:200,vertical:true,reset:Some(100),change:move |v|{let mut value=value;value.set(v);}}
        Slider {label:"Balance",value:balance(),min:-100,max:100,reset:Some(0),change:move |v|{let mut balance=balance;balance.set(v);}}
        div {class:"studio-mixer",div {class:"studio-mixer-channel",div {class:"studio-fader-bank",
            Slider {label:"Layout gain",value:100,min:0,max:200,vertical:true,change:move |_|{}}
            crate::audio_mixer::Meters {vertical:true,meter:acheflow_media_api::audio::AudioMeter::default()}
        }}}
    }
}
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..30 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("slider did not settle");
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
async fn sliders_support_keyboard_shared_values_and_bounded_vertical_drag() {
    let state = Rc::new(Harness {
        value: Cell::new(None),
        balance: Cell::new(None),
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
        window_size: (600, 600),
        ..Default::default()
    });
    settle(&mut doc);
    {
        let inner = doc.inner.borrow();
        let bank = inner.query_selector(".studio-fader-bank").unwrap().unwrap();
        let rows = inner
            .query_selector(".studio-fader-bank .audio-meters")
            .unwrap()
            .unwrap();
        let rows = inner
            .get_node(rows)
            .unwrap()
            .children
            .iter()
            .map(|&id| inner.get_node(id).unwrap())
            .filter(|n| n.element_data().is_some())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        let left = rows[0].absolute_position(0., 0.);
        let right = rows[1].absolute_position(0., 0.);
        assert!(
            right.x > left.x && (right.y - left.y).abs() < 1.,
            "L/R must stay beside the fader instead of wrapping: {left:?}, {right:?}"
        );
        assert!(
            inner.get_node(bank).unwrap().final_layout().size.height < 180.,
            "the level/fader bank must remain compact"
        );
    }
    let first = doc
        .inner
        .borrow()
        .query_selector("[aria-label='Gain']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(first);
    key(&mut doc, NamedKey::ArrowRight, Code::ArrowRight);
    assert_eq!(state.value.get().unwrap()(), 101);
    key(&mut doc, NamedKey::PageDown, Code::PageDown);
    assert_eq!(state.value.get().unwrap()(), 91);
    key(&mut doc, NamedKey::End, Code::End);
    assert_eq!(state.value.get().unwrap()(), 200);
    key(&mut doc, NamedKey::ArrowUp, Code::ArrowUp);
    assert_eq!(state.value.get().unwrap()(), 200);
    key(&mut doc, NamedKey::Home, Code::Home);
    assert_eq!(state.value.get().unwrap()(), 0);
    let rail = doc
        .inner
        .borrow()
        .query_selector("[aria-label='Vertical gain']")
        .unwrap()
        .unwrap();
    let (x, y) = {
        let inner = doc.inner.borrow();
        let node = inner.get_node(rail).unwrap();
        let point = node.absolute_position(18., 60.);
        (point.x, point.y)
    };
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, false)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(x, y, true)));
    settle(&mut doc);
    assert_eq!(
        state.value.get().unwrap()(),
        100,
        "clicking the midpoint sets unity gain"
    );
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x + 150., y - 80., true)));
    settle(&mut doc);
    assert_eq!(
        state.value.get().unwrap()(),
        200,
        "drag outside the narrow fader keeps updating and clamps"
    );
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x + 150., y + 100., true)));
    settle(&mut doc);
    assert_eq!(state.value.get().unwrap()(), 0);
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x + 150., y + 100., false)));
    settle(&mut doc);
    assert!(
        doc.inner
            .borrow()
            .query_selector(".slider-drag-shield")
            .unwrap()
            .is_none()
    );
    key(&mut doc, NamedKey::ArrowUp, Code::ArrowUp);
    assert_eq!(
        state.value.get().unwrap()(),
        1,
        "drag release must retain slider keyboard focus"
    );
    let mut value = state.value.get().unwrap();
    value.set(125);
    settle(&mut doc);
    {
        let inner = doc.inner.borrow();
        assert_eq!(
            inner
                .get_node(first)
                .unwrap()
                .attr(blitz_dom::local_name!("aria-valuenow")),
            Some("125")
        );
        assert_eq!(
            inner
                .get_node(rail)
                .unwrap()
                .attr(blitz_dom::local_name!("aria-valuenow")),
            Some("125")
        );
    }
    let pan = doc
        .inner
        .borrow()
        .query_selector("[aria-label='Balance']")
        .unwrap()
        .unwrap();
    doc.inner.borrow_mut().set_focus_to(pan);
    key(&mut doc, NamedKey::ArrowLeft, Code::ArrowLeft);
    assert_eq!(state.balance.get().unwrap()(), -1);
    key(&mut doc, NamedKey::Home, Code::Home);
    assert_eq!(state.balance.get().unwrap()(), -100);
    key(&mut doc, NamedKey::PageUp, Code::PageUp);
    assert_eq!(state.balance.get().unwrap()(), -90);
    let (x, y) = {
        let inner = doc.inner.borrow();
        let node = inner.get_node(pan).unwrap();
        let point = node.absolute_position(100., 14.);
        (point.x, point.y)
    };
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, false)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(x, y, true)));
    settle(&mut doc);
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, false)));
    settle(&mut doc);
    assert_eq!(
        state.balance.get().unwrap()(),
        0,
        "horizontal center restores centered stereo balance"
    );
    assert_eq!(
        state.value.get().unwrap()(),
        125,
        "another channel's slider remains unchanged"
    );
}
