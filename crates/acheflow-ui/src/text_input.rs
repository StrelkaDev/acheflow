//! Keep the Blitz element as the layout and form target while the platform edits it.
use blitz_dom::{BaseDocument, NodeId, ScrollBehavior, ScrollLogicalPosition};
use dioxus_native::{NodeHandle, prelude::*};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    node: NodeId,
    element: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct Insets {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(serde::Serialize)]
pub(crate) struct Field {
    #[serde(skip)]
    pub target: Target,
    pub rect: Rect,
    pub padding: Insets,
    pub label: String,
    pub placeholder: String,
    pub value: String,
    pub multiline: bool,
    pub code: bool,
    pub secret: bool,
    pub max_length: Option<usize>,
    pub input_mode: String,
    pub start: usize,
    pub end: usize,
}

pub(crate) struct Bridge {
    handle: NodeHandle,
    runtime: Rc<dioxus_core::Runtime>,
    scope: dioxus_core::ScopeId,
}

impl Bridge {
    pub fn new(handle: NodeHandle) -> Self {
        Self {
            handle,
            runtime: dioxus_core::Runtime::current(),
            scope: dioxus_core::current_scope_id(),
        }
    }

    #[cfg(target_os = "macos")]
    pub fn focused(&self) -> Option<Field> {
        let doc = self.handle.try_doc()?;
        field(&doc, doc.get_focussed_node_id()?)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn reveal_focused(&self) -> Option<Field> {
        let mut doc = self.handle.doc_mut();
        let id = doc.get_focussed_node_id()?;
        let target = Target {
            node: id,
            element: element_id(&doc, id)?,
        };
        if !valid(&doc, target) || hidden(&doc, id) {
            return None;
        }
        reveal(&mut doc, id);
        field(&doc, id)
    }

    #[cfg(target_os = "macos")]
    pub fn still_focused(&self, target: Target) -> bool {
        self.handle.try_doc().is_some_and(|doc| {
            doc.get_focussed_node_id() == Some(target.node) && valid(&doc, target)
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn at_point(&self, x: f32, y: f32, width: f32, height: f32) -> Option<Field> {
        let doc = self.handle.try_doc()?;
        if width <= 0. || height <= 0. {
            return None;
        }
        let viewport = doc.viewport();
        let x = x * viewport.window_size.0 as f32 / viewport.scale() / width;
        let y = y * viewport.window_size.1 as f32 / viewport.scale() / height;
        field(&doc, doc.element_from_point(x, y)?)
    }

    pub fn rect(&self, target: Target) -> Option<Rect> {
        let doc = self.handle.try_doc()?;
        valid(&doc, target)
            .then(|| rect(&doc, target.node))
            .flatten()
    }

    #[cfg(target_arch = "wasm32")]
    pub fn focus(&self, target: Target) {
        self.move_focus(|doc| {
            doc.set_focus_to(target.node);
        });
    }

    pub fn update(&self, target: Target, value: String) -> bool {
        let Some(doc) = self.handle.try_doc() else {
            return false;
        };
        if !valid(&doc, target) {
            return false;
        }
        drop(doc);
        self.handle
            .doc_mut()
            .with_text_input(target.node, |mut editor| {
                editor.editor.set_text(&value);
                // The platform editor can request a repaint before Dioxus updates the field.
                editor.refresh_layout();
            });
        self.form_event("input", target.element, value);
        true
    }

    pub fn finish(&self, target: Target, value: String, action: &str) {
        let Some(doc) = self.handle.try_doc() else {
            return;
        };
        if !valid(&doc, target) {
            return;
        }
        let focused = doc.get_focussed_node_id() == Some(target.node);
        let form = (action == "enter")
            .then(|| submit_form(&doc, target.node))
            .flatten();
        drop(doc);
        self.form_event("change", target.element, value.clone());
        if focused {
            self.move_focus(|doc| {
                if doc.get_focussed_node_id() != Some(target.node) {
                    return;
                }
                if !matches!(action, "tab" | "backtab")
                    || !advance_focus(doc, target.node, action == "backtab")
                {
                    doc.clear_focus();
                }
            });
        }
        if let Some(form) = form {
            self.form_event("submit", form, value);
        }
    }

    fn move_focus(&self, action: impl FnOnce(&mut BaseDocument)) {
        let (before, after) = {
            let mut doc = self.handle.doc_mut();
            let before = doc
                .get_focussed_node_id()
                .and_then(|id| element_id(&doc, id));
            action(&mut doc);
            let after = doc
                .get_focussed_node_id()
                .and_then(|id| element_id(&doc, id));
            (before, after)
        };
        if before == after {
            return;
        }
        if let Some(id) = before {
            self.event("blur", id, Box::new(()), false);
            self.event("focusout", id, Box::new(()), true);
        }
        if let Some(id) = after {
            self.event("focus", id, Box::new(()), false);
            self.event("focusin", id, Box::new(()), true);
        }
    }

    fn form_event(&self, name: &str, element: usize, value: String) {
        self.event(
            name,
            element,
            dioxus_native::synthetic_form_event(value, vec![]),
            true,
        );
    }

    fn event(&self, name: &str, element: usize, data: Box<dyn std::any::Any>, bubbles: bool) {
        self.runtime.in_scope(self.scope, || {
            let payload: Rc<dyn std::any::Any> = Rc::new(PlatformEventData::new(data));
            self.runtime.handle_event(
                name,
                Event::new(payload, bubbles),
                dioxus_core::ElementId::from_raw(element),
            );
        });
        self.handle.doc().shell_provider.request_redraw();
    }
}

fn element_id(doc: &BaseDocument, id: NodeId) -> Option<usize> {
    doc.get_node(id)?
        .attr("data-dioxus-id".into())?
        .parse()
        .ok()
}

fn valid(doc: &BaseDocument, target: Target) -> bool {
    doc.get_node(target.node).is_some_and(|node| {
        node.flags.is_in_document()
            && element_id(doc, target.node) == Some(target.element)
            && node.attr("disabled".into()).is_none()
            && node.attr("readonly".into()).is_none()
            && node
                .element_data()
                .and_then(|el| el.text_input_data())
                .is_some()
    })
}

fn submit_form(doc: &BaseDocument, mut id: NodeId) -> Option<usize> {
    while let Some(node) = doc.get_node(id) {
        if node.attr("data-text-submit".into()).is_some() {
            return element_id(doc, id);
        }
        id = node.parent?;
    }
    None
}

fn advance_focus(doc: &mut BaseDocument, from: NodeId, backwards: bool) -> bool {
    let Some(start) = doc.get_node(from) else {
        return false;
    };
    let candidate = |node: &blitz_dom::Node| {
        let size = node.final_layout().size;
        node.is_focussable() && size.width > 0. && size.height > 0. && !hidden(doc, node.id)
    };
    let next = if backwards {
        doc.prev_node(start, candidate)
    } else {
        doc.next_node(start, candidate)
    };
    let Some(next) = next else {
        return false;
    };
    doc.set_focus_to(next);
    reveal(doc, next);
    true
}

fn hidden(doc: &BaseDocument, mut id: NodeId) -> bool {
    while let Some(node) = doc.get_node(id) {
        if node.attr("aria-hidden".into()) == Some("true") {
            return true;
        }
        let Some(parent) = node.parent else {
            break;
        };
        id = parent;
    }
    false
}

fn reveal(doc: &mut BaseDocument, id: NodeId) {
    let offset = |start: f64, end: f64, clip_start: f64, clip_end: f64| {
        let a = start - clip_start;
        let b = end - clip_end;
        if a == 0. || b == 0. || a.is_sign_positive() != b.is_sign_positive() {
            0.
        } else if a.abs() < b.abs() {
            a
        } else {
            b
        }
    };
    let root = doc.try_root_element().map(|node| node.id);
    let mut ancestor = doc.get_node(id).and_then(|node| node.parent);
    while let Some(parent) = ancestor.filter(|parent| Some(*parent) != root) {
        let Some(node) = doc.get_node(parent) else {
            break;
        };
        ancestor = node.parent;
        let (Some(field), Some(pane)) = (bounds(doc, id), bounds(doc, parent)) else {
            continue;
        };
        let layout = node.final_layout();
        let left = pane.x + f64::from(layout.border.left);
        let top = pane.y + f64::from(layout.border.top);
        let right =
            pane.x + pane.width - f64::from(layout.border.right + layout.scrollbar_size.width);
        let bottom =
            pane.y + pane.height - f64::from(layout.border.bottom + layout.scrollbar_size.height);
        let x = offset(field.x, field.x + field.width, left, right);
        let y = offset(field.y, field.y + field.height, top, bottom);
        if x != 0. || y != 0. {
            // Blitz clamps attempts on non-scrollable ancestors to zero.
            doc.scroll_by(parent, x, y, ScrollBehavior::Instant);
        }
    }
    doc.scroll_into_view(
        id,
        ScrollBehavior::Instant,
        ScrollLogicalPosition::Nearest,
        ScrollLogicalPosition::Nearest,
    );
}

fn bounds(doc: &BaseDocument, id: NodeId) -> Option<Rect> {
    let layout = doc.get_client_bounding_rect(id)?;
    let offset = doc.get_node(id)?.scroll_offset();
    // The returned geometry subtracts the node's own scroll offset as well
    // as offsets from its ancestors. Only the latter move its border box.
    Some(Rect {
        x: layout.x + offset.x,
        y: layout.y + offset.y,
        width: layout.width,
        height: layout.height,
    })
}

fn rect(doc: &BaseDocument, id: NodeId) -> Option<Rect> {
    if hidden(doc, id) {
        return None;
    }
    let box_rect = bounds(doc, id)?;
    let viewport = doc.viewport();
    let width = f64::from(viewport.window_size.0) / f64::from(viewport.scale());
    let height = f64::from(viewport.window_size.1) / f64::from(viewport.scale());
    let x = box_rect.x.max(0.);
    let y = box_rect.y.max(0.);
    let right = (box_rect.x + box_rect.width).min(width);
    let bottom = (box_rect.y + box_rect.height).min(height);
    if right <= x || bottom <= y {
        return None;
    }
    let hit = doc.element_from_point(((x + right) * 0.5) as f32, ((y + bottom) * 0.5) as f32)?;
    if doc.nearest_non_anonymous_ancestor(hit)? != id {
        return None;
    }
    Some(box_rect)
}

fn field(doc: &BaseDocument, id: NodeId) -> Option<Field> {
    let id = doc.nearest_non_anonymous_ancestor(id)?;
    let target = Target {
        node: id,
        element: element_id(doc, id)?,
    };
    if !valid(doc, target) {
        return None;
    }
    let node = doc.get_node(id)?;
    let input = node.element_data()?.text_input_data()?;
    let value = input.editor.text().to_string();
    let selection = input.editor.raw_selection().text_range();
    let utf16_at = |byte| {
        value
            .get(..byte)
            .map_or(0, |prefix| prefix.encode_utf16().count())
    };
    let start = utf16_at(selection.start);
    let end = utf16_at(selection.end);
    let placeholder = node
        .attr("placeholder".into())
        .unwrap_or_default()
        .to_owned();
    let label = node
        .attr("aria-label".into())
        .map(str::to_owned)
        .or_else(|| {
            let label = doc.get_node(node.parent?)?;
            if label.element_data()?.name.local != blitz_dom::local_name!("label") {
                return None;
            }
            doc.get_node(*label.children.first()?)
                .map(|text| text.text_content())
        })
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| {
            if placeholder.is_empty() {
                "Edit field".into()
            } else {
                placeholder.clone()
            }
        });
    let padding = node.final_layout().padding;
    Some(Field {
        target,
        rect: rect(doc, id)?,
        padding: Insets {
            left: padding.left,
            right: padding.right,
            top: padding.top,
            bottom: padding.bottom,
        },
        label,
        placeholder,
        value,
        multiline: input.is_multiline,
        code: node.attr("class".into()).is_some_and(|classes| {
            classes
                .split_whitespace()
                .any(|class| class == "code-input")
        }),
        secret: node.attr("type".into()) == Some("password"),
        max_length: node.attr("maxlength".into()).and_then(|v| v.parse().ok()),
        input_mode: node
            .attr("inputmode".into())
            .unwrap_or(if node.attr("type".into()) == Some("number") {
                "decimal"
            } else {
                "text"
            })
            .to_owned(),
        start,
        end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_dom::{Document, DocumentConfig};
    use blitz_traits::shell::Viewport;
    use dioxus_native::DioxusDocument;
    use std::cell::RefCell;

    fn fields() -> Element {
        rsx! {div { id:"pane", style:"overflow:auto;width:280px;height:90px;",
            input {id:"first",r#type:"text",value:"first",oninput:move |_|{}}
            div { "aria-hidden":"true", style:"visibility:hidden;height:180px;",
                input {id:"hidden",r#type:"text",value:"hidden",oninput:move |_|{}}
            }
            input {id:"last",r#type:"text",value:"last",oninput:move |_|{}}
        }}
    }

    #[tokio::test]
    async fn platform_input_is_ready_to_paint_before_the_next_dom_update() {
        type Mounted = Rc<RefCell<Option<Bridge>>>;

        fn app() -> Element {
            let mounted = use_context::<Mounted>();
            rsx! {div {
                onmounted: move |event| {
                    let handle = event.data().downcast::<NodeHandle>().unwrap().clone();
                    mounted.replace(Some(Bridge::new(handle)));
                },
                input {id:"size",r#type:"text",value:"1280",oninput:move |_|{}}
            }}
        }

        let mounted = Mounted::default();
        let mut document = DioxusDocument::new(
            VirtualDom::new(app).with_root_context(mounted.clone()),
            DocumentConfig::default(),
        );
        document.initial_build();
        for _ in 0..10 {
            if !document.poll(None) {
                break;
            }
        }
        let target = {
            let mut doc = document.inner.borrow_mut();
            doc.resolve(0.);
            let node = doc.query_selector("#size").unwrap().unwrap();
            Target {
                node,
                element: element_id(&doc, node).unwrap(),
            }
        };
        assert!(
            mounted
                .borrow()
                .as_ref()
                .unwrap()
                .update(target, "1920".into())
        );
        let doc = document.inner.borrow();
        let node = doc.get_node(target.node).unwrap();
        assert_eq!(
            node.element_data()
                .unwrap()
                .text_input_data()
                .unwrap()
                .editor
                .text(),
            "1920"
        );
        assert!(
            node.text_input_v_centering_offset(f64::from(doc.viewport().scale()))
                .is_finite()
        );
    }

    #[tokio::test]
    async fn revealing_tab_targets_scrolls_the_nested_pane_both_ways() {
        let mut document = DioxusDocument::new(VirtualDom::new(fields), DocumentConfig::default());
        document.initial_build();
        document.inner.borrow_mut().set_viewport(Viewport {
            window_size: (420, 320),
            ..Default::default()
        });
        for _ in 0..10 {
            if !document.poll(None) {
                break;
            }
        }
        let mut doc = document.inner.borrow_mut();
        doc.resolve(0.);
        let first = doc.query_selector("#first").unwrap().unwrap();
        let last = doc.query_selector("#last").unwrap().unwrap();
        let pane = doc.query_selector("#pane").unwrap().unwrap();
        let before = doc.get_client_bounding_rect(last).unwrap();
        let bounds = doc.get_client_bounding_rect(pane).unwrap();
        assert!(
            before.y >= bounds.y + bounds.height,
            "{before:?} should be below {bounds:?}"
        );

        assert!(doc.set_focus_to(first));
        assert!(advance_focus(&mut doc, first, false));
        assert_eq!(doc.get_focussed_node_id(), Some(last));
        assert!(doc.get_node(pane).unwrap().scroll_offset().y > 0.);
        let after = doc.get_client_bounding_rect(last).unwrap();
        assert!(after.y + after.height <= bounds.y + bounds.height + 1.);

        assert!(advance_focus(&mut doc, last, true));
        assert_eq!(doc.get_focussed_node_id(), Some(first));
        assert_eq!(doc.get_node(pane).unwrap().scroll_offset().y, 0.);
        assert!(rect(&doc, first).is_some());
    }
}
