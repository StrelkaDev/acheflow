//! Positioned browser editors for the Blitz canvas.
use crate::text_input::{Bridge, Target};
use dioxus_native::{NodeHandle, prelude::*};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(module = "/src/mobile_input.js")]
extern "C" {
    #[wasm_bindgen(js_name=installTextInput)]
    fn install(
        open: &js_sys::Function,
        position: &js_sys::Function,
        edit: &js_sys::Function,
    ) -> js_sys::Function;
}

struct BrowserEditor {
    cleanup: js_sys::Function,
    _open: Closure<dyn FnMut(f32, f32, f32, f32) -> String>,
    _position: Closure<dyn FnMut() -> String>,
    _edit: Closure<dyn FnMut(String, String) -> bool>,
}

impl Drop for BrowserEditor {
    fn drop(&mut self) {
        let _ = self.cleanup.call0(&JsValue::NULL);
    }
}

#[component]
pub(crate) fn Mount() -> Element {
    let editor = use_hook(|| Rc::new(RefCell::new(None::<BrowserEditor>)));
    let teardown = editor.clone();
    use_drop(move || {
        teardown.borrow_mut().take();
    });
    rsx! {div {style:"display:none",onmounted:move|event|{
        if editor.borrow().is_some() {return;}
        let Some(handle) = event.data().downcast::<NodeHandle>().cloned() else {return;};
        let bridge = Rc::new(Bridge::new(handle));
        let active = Rc::new(std::cell::Cell::new(None::<Target>));
        let open = {
            let (bridge, active) = (bridge.clone(), active.clone());
            Closure::<dyn FnMut(f32, f32, f32, f32) -> String>::new(move |x,y,w,h| {
                let field = if w == 0. {bridge.reveal_focused()} else {bridge.at_point(x,y,w,h)};
                let Some(field) = field else {return String::new();};
                bridge.focus(field.target);
                active.set(Some(field.target));
                serde_json::to_string(&field).unwrap_or_default()
            })
        };
        let position = {
            let (bridge, active) = (bridge.clone(), active.clone());
            Closure::<dyn FnMut() -> String>::new(move || {
                active.get().and_then(|target| bridge.rect(target))
                    .and_then(|rect| serde_json::to_string(&rect).ok()).unwrap_or_default()
            })
        };
        let edit = Closure::<dyn FnMut(String, String) -> bool>::new(move |action:String,value:String| {
            let Some(target) = active.get() else {return false;};
            if action == "input" {
                bridge.update(target, value)
            } else {
                active.set(None);
                bridge.finish(target, value, &action);
                true
            }
        });
        let cleanup = install(open.as_ref().unchecked_ref(), position.as_ref().unchecked_ref(), edit.as_ref().unchecked_ref());
        *editor.borrow_mut() = Some(BrowserEditor { cleanup, _open:open, _position:position, _edit:edit });
    }}}
}
