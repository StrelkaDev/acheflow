//! Rails stay visible while their axis overflows. Blitz's native overlay
//! scrollbars fade to zero at rest, which makes code overflow invisible.
use dioxus_native::{NodeHandle, prelude::*};

const RAIL_SIZE: f64 = 14.;
const OVERFLOW_TOLERANCE: f64 = 0.5;

#[derive(Clone, Copy, Default, PartialEq)]
struct Metrics {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    content_width: f64,
    content_height: f64,
    frame_width: f64,
    frame_height: f64,
}
impl Metrics {
    fn overflow(self) -> (bool, bool) {
        if self.frame_width <= 0. || self.frame_height <= 0. {
            return (false, false);
        }
        // Content with min-width:100% can simply be filling the old viewport.
        // Treat it as intrinsic width only when it actually overflows that view.
        let width = if self.content_width > self.width + OVERFLOW_TOLERANCE {
            self.content_width
        } else {
            0.
        };
        let mut horizontal = width > self.frame_width + OVERFLOW_TOLERANCE;
        let mut vertical = self.content_height > self.frame_height + OVERFLOW_TOLERANCE;
        // Reserving one rail can make the other axis overflow. Start from the
        // full frame so mutually reserved gutters never keep obsolete rails.
        for _ in 0..2 {
            horizontal |= width
                > self.frame_width - if vertical { RAIL_SIZE } else { 0. } + OVERFLOW_TOLERANCE;
            vertical |= self.content_height
                > self.frame_height - if horizontal { RAIL_SIZE } else { 0. } + OVERFLOW_TOLERANCE;
        }
        (horizontal, vertical)
    }
    fn axis(self, horizontal: bool) -> (f64, f64, f64) {
        if horizontal {
            (self.x, self.width, self.content_width)
        } else {
            (self.y, self.height, self.content_height)
        }
    }
}
#[derive(Clone, Copy)]
struct Drag {
    horizontal: bool,
    start: f64,
    offset: f64,
    ratio: f64,
}

fn refresh(handle: Signal<Option<NodeHandle>>, mut metrics: Signal<Metrics>) {
    if let Some(handle) = handle.peek().as_ref()
        && let Some(doc) = handle.try_doc()
        && let Some(node) = doc.get_node(handle.node_id())
    {
        let position = *node.scroll_offset();
        let Some(frame) = node.parent.and_then(|id| doc.get_node(id)) else {
            return;
        };
        // Hidden inspectors retain their viewport and scroll position.
        if frame.client_width() <= 0. || frame.client_height() <= 0. {
            return;
        }
        let mut next = Metrics {
            x: position.x,
            y: position.y,
            width: f64::from(node.client_width()),
            height: f64::from(node.client_height()),
            content_width: f64::from(node.scroll_width()),
            content_height: f64::from(node.scroll_height()),
            frame_width: f64::from(frame.client_width()),
            frame_height: f64::from(frame.client_height()),
        };
        // Blitz does not clamp the old offset when the text becomes shorter.
        // Clamp after layout, before hiding a now-unnecessary rail.
        let range = |content: f64, view: f64| {
            if content > view + OVERFLOW_TOLERANCE {
                content - view
            } else {
                0.
            }
        };
        next.x = next.x.clamp(0., range(next.content_width, next.width));
        next.y = next.y.clamp(0., range(next.content_height, next.height));
        drop(doc);
        if (next.x, next.y) != (position.x, position.y) {
            handle.doc_mut().scroll_to(
                handle.node_id(),
                next.x,
                next.y,
                blitz_dom::ScrollBehavior::Instant,
            );
        }
        if next != *metrics.peek() {
            metrics.set(next);
        }
    }
}
fn scroll(
    handle: Signal<Option<NodeHandle>>,
    metrics: Signal<Metrics>,
    horizontal: bool,
    offset: f64,
) {
    if let Some(handle) = handle.peek().as_ref() {
        let position = *metrics.peek();
        handle.doc_mut().scroll_to(
            handle.node_id(),
            if horizontal { offset } else { position.x },
            if horizontal { position.y } else { offset },
            blitz_dom::ScrollBehavior::Instant,
        );
    }
    refresh(handle, metrics);
}
#[component]
pub fn CodeScroll(width: String, height: usize, children: Element) -> Element {
    let mut handle = use_signal(|| None::<NodeHandle>);
    let metrics = use_signal(Metrics::default);
    let mut drag = use_signal(|| None::<Drag>);
    // Mounted runs before Blitz layout. Recheck after layout/resize without
    // forcing a second document resolve or changing the animation timeline.
    use_future(move || async move {
        loop {
            refresh(handle, metrics);
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(100).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
    let update = move || {
        spawn(async move {
            refresh(handle, metrics);
        });
    };
    let (horizontal_overflow, vertical_overflow) = metrics().overflow();
    rsx! {div {class:format!("code-frame {} {}",if horizontal_overflow{"has-horizontal"}else{""},if vertical_overflow{"has-vertical"}else{""}),
        onmousemove:move |e| {
            if let Some(d)=*drag.peek() {
                let p=e.client_coordinates();
                let delta=if d.horizontal{p.x}else{p.y}-d.start;
                spawn(async move{scroll(handle,metrics,d.horizontal,d.offset+delta*d.ratio);});
            }else{update();}
        },
        onmouseup:move |_|drag.set(None),onmouseleave:move |_|drag.set(None),
        onkeyup:move |_|update(),
        div {class:"code-scroll",tabindex:"-1",
            onmounted:move |e|{handle.set(e.data().downcast::<NodeHandle>().cloned());update();},
            onscroll:move |_|update(),
            div {class:"code-content",style:format!("width:{width};height:{height}px;"),{children}}
        }
        for horizontal in [false,true].into_iter().filter(|h|if *h{horizontal_overflow}else{vertical_overflow}) {
            {let (offset,view,content)=metrics().axis(horizontal);
            let fraction=if content>0.{(view/content).clamp(0.03,1.)}else{1.};
            let position=if content>view && view>0. {offset/(content-view)*(1.-fraction)}else{0.};
            rsx! {div {class:if horizontal{"code-rail horizontal"}else{"code-rail vertical"},
                role:"scrollbar",tabindex:"0","aria-label":if horizontal{"Code horizontal scroll"}else{"Code vertical scroll"},
                "aria-orientation":if horizontal{"horizontal"}else{"vertical"},"aria-valuemin":"0","aria-valuemax":(content-view).max(0.).round().to_string(),"aria-valuenow":offset.round().to_string(),
                onmousedown:move |e| {e.prevent_default();let p=e.element_coordinates();let value=if horizontal{p.x}else{p.y};spawn(async move{scroll(handle,metrics,horizontal,(value/view.max(1.))*(content-view).max(0.));});},
                onkeydown:move |e| {
                    let key=e.key();
                    if !matches!(key,Key::Named(NamedKey::ArrowLeft|NamedKey::ArrowUp|NamedKey::ArrowRight|NamedKey::ArrowDown|NamedKey::PageUp|NamedKey::PageDown|NamedKey::Home|NamedKey::End)){return;}
                    e.prevent_default();spawn(async move {
                        refresh(handle,metrics);
                        let (offset,view,content)=metrics.peek().axis(horizontal);
                        let next=match key{Key::Named(NamedKey::ArrowLeft|NamedKey::ArrowUp)=>offset-40.,Key::Named(NamedKey::ArrowRight|NamedKey::ArrowDown)=>offset+40.,Key::Named(NamedKey::PageUp)=>offset-view,Key::Named(NamedKey::PageDown)=>offset+view,Key::Named(NamedKey::Home)=>0.,_=>(content-view).max(0.)};
                        scroll(handle,metrics,horizontal,next);
                    });
                },
                div {class:"code-thumb",style:if horizontal{format!("left:{}%;width:{}%;",position*100.,fraction*100.)}else{format!("top:{}%;height:{}%;",position*100.,fraction*100.)},
                    onmousedown:move |e|{e.prevent_default();e.stop_propagation();let p=e.client_coordinates();drag.set(Some(Drag {horizontal,start:if horizontal{p.x}else{p.y},offset,ratio:if fraction<1.{(content-view)/(view*(1.-fraction)).max(1.)}else{0.}}));}
                }
            }}}
        }
    }}
}
