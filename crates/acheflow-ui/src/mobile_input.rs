//! Bridge touch editing into a real DOM input; winit-web currently has no IME.
use dioxus_native::NodeHandle;
use dioxus_native::prelude::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};
#[wasm_bindgen(module = "/src/mobile_input.js")]
extern "C" {
    #[wasm_bindgen(js_name=installMobileInput)]
    fn install(open: &js_sys::Function, update: &js_sys::Function) -> js_sys::Function;
}
struct Active {
    node: blitz_dom::NodeId,
    element: usize,
    token: u32,
}
struct Bridge {
    cleanup: js_sys::Function,
    _open: Closure<dyn FnMut(f32, f32, f32, f32) -> String>,
    _update: Closure<dyn FnMut(u32, String, bool) -> bool>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.cleanup.call0(&JsValue::NULL);
    }
}
#[component]
pub(crate) fn Mount() -> Element {
    let bridge = use_hook(|| Rc::new(RefCell::new(None::<Bridge>)));
    let cleanup = bridge.clone();
    use_drop(move || {
        cleanup.borrow_mut().take();
    });
    let runtime = dioxus_core::Runtime::current();
    let scope = dioxus_core::current_scope_id();
    rsx! {div{style:"display:none",onmounted:move|e|{
        if bridge.borrow().is_some(){return;}
        let Some(handle)=e.data().downcast::<NodeHandle>().cloned() else{return;};
        let active=Rc::new(RefCell::new(None::<Active>));
        let current=active.clone();let root=handle.clone();let mut token=0u32;
        let open=Closure::wrap(Box::new(move|x:f32,y:f32,w:f32,h:f32|->String{
            let Some(doc)=root.try_doc() else{return String::new();};
            if w<=0. || h<=0. {return String::new();}
            let viewport=doc.viewport();let scale=viewport.scale();
            let Some(id)=doc.element_from_point(x*viewport.window_size.0 as f32/scale/w,y*viewport.window_size.1 as f32/scale/h) else{return String::new();};
            let Some(node)=doc.get_node(id) else{return String::new();};
            if node.attr("disabled".into()).is_some()||node.attr("readonly".into()).is_some(){return String::new();}
            let Some(input)=node.element_data().and_then(|e|e.text_input_data()) else{return String::new();};
            let Some(element)=node.attr("data-dioxus-id".into()).and_then(|v|v.parse::<usize>().ok()) else{return String::new();};
            let mut label=node.attr("aria-label".into()).or_else(||node.attr("placeholder".into())).unwrap_or("Edit field").to_owned();
            if let Some(first)=node.parent.and_then(|id|doc.get_node(id)).and_then(|parent|parent.children.first()).and_then(|id|doc.get_node(*id)) {
                let text=first.text_content();if !text.trim().is_empty(){label=text;}
            }
            token=token.wrapping_add(1);
            let descriptor=serde_json::json!({"token":token,"label":label,"value":input.editor.text().to_string(),"multiline":input.is_multiline,"secret":node.attr("type".into())==Some("password"),"max_length":node.attr("maxlength".into()).and_then(|v|v.parse::<u32>().ok()).unwrap_or(8192),"input_mode":node.attr("inputmode".into()).unwrap_or(if node.attr("type".into())==Some("number"){"decimal"}else{"text"})});
            *current.borrow_mut()=Some(Active{node:id,element,token});
            descriptor.to_string()
        }) as Box<dyn FnMut(f32,f32,f32,f32)->String>);
        let runtime=runtime.clone();
        let update=Closure::wrap(Box::new(move|token:u32,value:String,change:bool|->bool{
            let active=active.borrow();let Some(active)=active.as_ref().filter(|a|a.token==token) else{return false;};
            let Some(doc)=handle.try_doc() else{return false;};
            let valid=doc.get_node(active.node).is_some_and(|n|n.attr("data-dioxus-id".into()).and_then(|v|v.parse::<usize>().ok())==Some(active.element)&&n.attr("disabled".into()).is_none()&&n.attr("readonly".into()).is_none());
            let shell=doc.shell_provider.clone();drop(doc);if !valid{return false;}
            runtime.in_scope(scope,||{
                let payload:Rc<dyn std::any::Any>=Rc::new(PlatformEventData::new(dioxus_native::synthetic_form_event(value,vec![])));
                runtime.handle_event(if change{"change"}else{"input"},Event::new(payload,true),dioxus_core::ElementId::from_raw(active.element));
            });
            shell.request_redraw();true
        }) as Box<dyn FnMut(u32,String,bool)->bool>);
        let cleanup=install(open.as_ref().unchecked_ref(),update.as_ref().unchecked_ref());
        *bridge.borrow_mut()=Some(Bridge{cleanup,_open:open,_update:update});
    }}}
}
