//! Shared native/WASM slider: Blitz does not implement HTML range controls.
use dioxus_native::prelude::dioxus_html::input_data::MouseButton;
use dioxus_native::{NodeHandle, prelude::*};

const INSET: f64 = 10.;
#[derive(Clone, Copy)]
struct Rail {
    start: f64,
    length: f64,
}
fn snap(value: f64, min: i32, max: i32, step: i32) -> i32 {
    let step = f64::from(step.max(1));
    (f64::from(min) + ((value - f64::from(min)) / step).round() * step)
        .clamp(f64::from(min), f64::from(max)) as i32
}
fn point_value(point: f64, rail: Rail, vertical: bool, min: i32, max: i32, step: i32) -> i32 {
    let fraction =
        ((point - rail.start - INSET) / (rail.length - 2. * INSET).max(1.)).clamp(0., 1.);
    let fraction = if vertical { 1. - fraction } else { fraction };
    snap(
        f64::from(min) + fraction * f64::from(max - min),
        min,
        max,
        step,
    )
}

#[component]
pub(crate) fn Slider(
    label: String,
    value: i32,
    min: i32,
    max: i32,
    change: EventHandler<i32>,
    #[props(default = 1)] step: i32,
    #[props(default)] unit: String,
    #[props(default = false)] vertical: bool,
    #[props(default)] reset: Option<i32>,
) -> Element {
    let mut handle = use_signal(|| None::<NodeHandle>);
    let mut current = use_signal(|| value.clamp(min, max));
    let mut rail = use_signal(|| None::<Rail>);
    let mut dragging = use_signal(|| false);
    let mut generation = use_signal(|| 0u64);
    use_effect(use_reactive(
        (&value, &min, &max),
        move |(value, min, max)| current.set(value.clamp(min, max)),
    ));
    let mut update = move |value| {
        let next = snap(f64::from(value), min, max, step);
        if *current.peek() != next {
            current.set(next);
            change.call(next);
        }
    };
    let fraction = if max > min {
        f64::from(current() - min) / f64::from(max - min)
    } else {
        0.
    };
    let position = fraction.clamp(0., 1.) * 100.;
    rsx! {div {class:if vertical {"value-slider vertical-slider"}else{"value-slider"},
        div {class:"slider-caption",span {"{label}"}if !vertical {span {class:"slider-readout","{current}{unit}"}}}
        div {class:if vertical {"slider-control vertical"}else{"slider-control"},role:"slider",tabindex:0,
            "aria-label":label.clone(),"aria-valuemin":min.to_string(),"aria-valuemax":max.to_string(),"aria-valuenow":current().to_string(),"aria-valuetext":format!("{}{unit}",current()),"aria-orientation":if vertical {"vertical"}else{"horizontal"},
            title:format!("{label}: {}{unit}. Arrow keys: one step; Page Up/Down: ten steps.",current()),
            onmounted:move |e|handle.set(e.data().downcast::<NodeHandle>().cloned()),
            onpointerdown:move |e|{
                if !e.is_primary() || e.trigger_button().is_some_and(|b|b!=MouseButton::Primary){return;}
                e.prevent_default();e.stop_propagation();
                dragging.set(true);generation+=1;let revision=*generation.peek();
                let p=e.client_coordinates();let point=if vertical {p.y}else{p.x};
                // Query after event dispatch releases Blitz's document borrow.
                // A fast release still updates the clicked value, without reviving a drag.
                spawn(async move {
                    let bounds=handle.peek().as_ref().and_then(|handle|handle.doc().get_client_bounding_rect(handle.node_id()));
                    if *generation.peek()!=revision {return;}
                    if let Some(bounds)=bounds {
                        if let Some(handle)=handle.peek().as_ref(){handle.doc_mut().set_focus_to(handle.node_id());}
                        let next=Rail{start:if vertical {bounds.y}else{bounds.x},length:if vertical {bounds.height}else{bounds.width}};
                        rail.set(Some(next));update(point_value(point,next,vertical,min,max,step));
                    }else{dragging.set(false);}
                });
            },
            onpointermove:move |e|{
                if *dragging.peek() && let Some(rail)=*rail.peek() {
                    e.prevent_default();e.stop_propagation();let p=e.client_coordinates();
                    update(point_value(if vertical {p.y}else{p.x},rail,vertical,min,max,step));
                }
            },
            onpointerup:move |e|{
                e.prevent_default();e.stop_propagation();dragging.set(false);
                // Removing the drag shield can clear Blitz focus; restore the rail
                // after the release/default click has finished, so arrow keys work.
                spawn(async move {if let Some(handle)=handle.peek().as_ref(){handle.doc_mut().set_focus_to(handle.node_id());}});
            },onpointercancel:move |_|dragging.set(false),onblur:move |_|dragging.set(false),
            onkeydown:move |e|{
                let key=e.key();let value=*current.peek();
                let next=match key {
                    Key::Named(NamedKey::ArrowUp|NamedKey::ArrowRight)=>Some(value.saturating_add(step.max(1))),
                    Key::Named(NamedKey::ArrowDown|NamedKey::ArrowLeft)=>Some(value.saturating_sub(step.max(1))),
                    Key::Named(NamedKey::PageUp)=>Some(value.saturating_add(step.max(1).saturating_mul(10))),
                    Key::Named(NamedKey::PageDown)=>Some(value.saturating_sub(step.max(1).saturating_mul(10))),
                    Key::Named(NamedKey::Home)=>Some(min),Key::Named(NamedKey::End)=>Some(max),
                    Key::Named(NamedKey::Escape)=>{dragging.set(false);None},_=>None,
                };
                if let Some(next)=next {e.prevent_default();e.stop_propagation();update(next);}
            },
            div {class:"slider-lane",
                div {class:"slider-fill",style:format!("{}:{position:.2}%;",if vertical {"height"}else{"width"})}
                div {class:"slider-thumb",style:format!("{}:calc({position:.2}% - {}px);",if vertical {"bottom"}else{"left"},if vertical {5}else{7})}
            }
            if dragging() {div {class:"slider-drag-shield"}}
        }
        if vertical {span {class:"slider-readout","{current}{unit}"}}
        if let Some(reset)=reset {button {class:"slider-reset",title:format!("Reset {label} to {reset}{unit}"),"aria-label":format!("Reset {label} to {reset}{unit}"),onclick:move |_|update(reset),"↺"}}
    }}
}
