//! Real layer-inspector regression without a browser or GPU adapter.
use crate::{
    layer_editor::LayerEditor,
    live_client::use_live_client,
    project::{Orientation, Workspace},
};
use blitz_dom::{Document, DocumentConfig};
use blitz_traits::events::UiEvent;
use dioxus_native::{DioxusDocument, prelude::*};
use std::{cell::Cell, rc::Rc};

struct Harness {
    initial: Workspace,
    scene: u32,
    first: u64,
    project: Cell<Option<Signal<Workspace>>>,
    selection: Cell<Option<Signal<Option<u64>>>>,
}

fn app() -> Element {
    let state = consume_context::<Rc<Harness>>();
    let project = use_signal(|| state.initial.clone());
    let selection = use_signal(|| Some(state.first));
    use_live_client(project);
    state.project.set(Some(project));
    state.selection.set(Some(selection));
    rsx! { LayerEditor {project, scene_id:state.scene, selection} }
}

fn settle(doc: &mut DioxusDocument) {
    for _ in 0..30 {
        if !doc.poll(None) {
            doc.inner.borrow_mut().resolve(0.);
            return;
        }
    }
    panic!("layer editor did not become idle");
}

const ACTIVE_CSS: &str = ".layer-inspector[style='display:block'] textarea";

fn active_text(doc: &DioxusDocument) -> String {
    let inner = doc.inner.borrow();
    let id = inner.query_selector(ACTIVE_CSS).unwrap().unwrap();
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

fn paste(doc: &mut DioxusDocument, text: &str) {
    let id = doc
        .inner
        .borrow()
        .query_selector(ACTIVE_CSS)
        .unwrap()
        .unwrap();
    {
        let mut inner = doc.inner.borrow_mut();
        inner.set_focus_to(id);
        inner.with_text_input(id, |mut editor| editor.select_all());
    }
    doc.handle_ui_event(UiEvent::Paste(text.into()));
    settle(doc);
    assert_eq!(active_text(doc), text);
}

#[tokio::test]
async fn real_inspector_retains_each_same_source_layer_draft_after_selection_and_reorder() {
    let mut initial = Workspace::default();
    initial.add_source();
    let scene = initial.add_scene(Orientation::Horizontal);
    // Two layers of the same source still have independent editor ownership.
    initial.add_layer(scene, initial.sources[0].id);
    initial.add_layer(scene, initial.sources[0].id);
    let first = initial.scenes[0].layers[0].id;
    let second = initial.scenes[0].layers[1].id;
    let originals = initial.scenes[0].layers.clone();
    let state = Rc::new(Harness {
        initial,
        scene,
        first,
        project: Cell::new(None),
        selection: Cell::new(None),
    });
    let vdom = VirtualDom::new(app).with_root_context(state.clone());
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                "../assets/DejaVuSans.woff2"
            ))),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    paste(&mut doc, ".layer { background: url(");
    let mut selection = state.selection.get().unwrap();
    selection.set(Some(second));
    settle(&mut doc);
    paste(&mut doc, ".layer { opacity:");

    let mut project = state.project.get().unwrap();
    assert!(project.write().move_layer(scene, 0, 1));
    selection.set(Some(first));
    settle(&mut doc);
    assert_eq!(active_text(&doc), ".layer { background: url(");
    selection.set(Some(second));
    settle(&mut doc);
    assert_eq!(active_text(&doc), ".layer { opacity:");

    for original in originals {
        let workspace = project.read();
        let layer = workspace.scenes[0]
            .layers
            .iter()
            .find(|l| l.id == original.id)
            .unwrap();
        assert_eq!(
            layer.css, original.css,
            "unapplied text must not reach another layer or the applied scene"
        );
    }
}
