use acheflow_media_api::appearance::{validate_css, validate_shader};
use dioxus_native::prelude::*;
#[path = "code_scroll.rs"]
mod scrolling;
#[path = "code_syntax.rs"]
mod syntax;
#[derive(Clone, Copy, PartialEq)]
pub enum CodeKind {
    Css,
    Wgsl,
}
impl CodeKind {
    fn validate(self, text: &str) -> Result<(), String> {
        match self {
            Self::Css => validate_css(text),
            Self::Wgsl => validate_shader(text),
        }
    }
}

#[component]
pub fn CodeEditor(
    label: &'static str,
    value: String,
    kind: CodeKind,
    change: EventHandler<String>,
) -> Element {
    let mut draft = use_signal(|| value.clone());
    let mut revision = use_signal(|| 0u64);
    let mut applied = use_signal(|| value.clone());
    let mut error = use_signal(|| None::<String>);
    use_effect(use_reactive((&value,), move |(value,)| {
        // Props may refresh after reordering or an external geometry edit. Only
        // replace a clean buffer; an invalid/unapplied draft belongs to this editor.
        if *draft.peek() == *applied.peek() {
            draft.set(value.clone());
            revision += 1;
            error.set(None);
        }
        applied.set(value);
    }));
    let commit = move || {
        let text = draft.peek().clone();
        match kind.validate(&text) {
            Ok(()) => {
                applied.set(text.clone());
                change.call(text);
                error.set(None);
            }
            Err(e) => error.set(Some(e)),
        }
    };
    rsx! {div {class:"code-editor",
        CodeSurface {label,kind,draft,revision,
            input:move |text|{draft.set(text);if error.peek().is_some(){error.set(None);}},
            blur:move |_|{let mut commit=commit;commit();},
        }
        div {class:"row",button {onclick:move |_|{let mut commit=commit;commit();},"Update layer"}
            button {onclick:move |_|{spawn(async move {
                let extension=if kind==CodeKind::Css{"css"}else{"wgsl"};
                if let Some(file)=rfd::AsyncFileDialog::new().add_filter("Layer code",&[extension]).pick_file().await {
                    let bytes=file.read().await;
                    if bytes.len()>32768 {error.set(Some("Code file is too large".into()));return;}
                    match String::from_utf8(bytes).map_err(|e|e.to_string()).and_then(|text|kind.validate(&text).map(|_|text)) {
                        Ok(text)=>{draft.set(text.clone());applied.set(text.clone());revision += 1;change.call(text);error.set(None);},Err(e)=>error.set(Some(e)),
                    }
                }
            });},"Load file"}
            span {class:"form-help","Update applies to the editor; Apply scene & sources updates the server."}
        }
        if let Some(error)=error.read().as_ref(){pre {class:"code-error",role:"alert","{error}"}}
    }}
}

// Highlight/layout updates never rewrite the native text editor on every key.
// Its volatile value is read from the latest buffer only when this input renders.
#[component]
fn CodeInput(
    label: &'static str,
    draft: Signal<String>,
    revision: Signal<u64>,
    input: EventHandler<String>,
    blur: EventHandler<()>,
) -> Element {
    let _ = revision();
    let initial = draft.peek().clone();
    let mut mounted = use_signal(|| None::<dioxus_native::NodeHandle>);
    rsx! {textarea {class:"code-input",rows:12,spellcheck:"false",wrap:"off","aria-label":label,value:initial.clone(),
        onmounted:move |e|mounted.set(e.data().downcast::<dioxus_native::NodeHandle>().cloned()),
        oninput:move |e|{input.call(e.value());if let Some(handle)=mounted.peek().clone(){spawn(async move{follow_caret(&handle);});}},
        onkeyup:move |_|{if let Some(handle)=mounted.peek().clone(){spawn(async move{follow_caret(&handle);});}},
        onblur:move |_|blur.call(()),
        "{initial}"
    }}
}
#[component]
fn CodeSurface(
    label: &'static str,
    kind: CodeKind,
    draft: Signal<String>,
    revision: Signal<u64>,
    input: EventHandler<String>,
    blur: EventHandler<()>,
) -> Element {
    let text = draft.read();
    let lines = text.split('\n').count().max(12);
    let columns = text
        .split('\n')
        .map(|l| {
            l.chars()
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
        + 2;
    rsx! {label {"{label}",
        scrolling::CodeScroll {width:format!("max(100%, calc({columns}ch + 32px))"),height:lines*18+32,
                CodeInput {label,draft,revision,input,blur}
                // Paint glyphs above native selection without a stacking context:
                // Blitz must keep this layer inside the scroller's clip.
                pre {class:"code-highlight","aria-hidden":"true",for (text,class) in syntax::tokens(&text,kind) {span {class,"{text}"}}}
        }
    }}
}
fn follow_caret(handle: &dioxus_native::NodeHandle) {
    let mut doc = handle.doc_mut();
    let Some(node) = doc.get_node(handle.node_id()) else {
        return;
    };
    if !node.is_focussed() {
        return;
    }
    let Some(editor) = node.element_data().and_then(|e| e.text_input_data()) else {
        return;
    };
    let Some(caret) = editor.editor.cursor_geometry(1.5) else {
        return;
    };
    let scale = doc.viewport().scale_f64();
    let Some(content) = node.parent else {
        return;
    };
    let Some(scroll) = doc.get_node(content).and_then(|n| n.parent) else {
        return;
    };
    let Some(view) = doc.get_node(scroll) else {
        return;
    };
    let layout = view.final_layout();
    let old = *view.scroll_offset();
    let adjust = |offset: f64, start: f64, end: f64, extent: f64| {
        if end > offset + extent - 16. {
            (end - extent + 16.).max(0.)
        } else if start < offset {
            start.max(0.)
        } else {
            offset
        }
    };
    let x = adjust(
        old.x,
        caret.x0 / scale + 16.,
        caret.x1 / scale + 16.,
        layout.size.width as f64,
    );
    let y = adjust(
        old.y,
        caret.y0 / scale + 16.,
        caret.y1 / scale + 16.,
        layout.size.height as f64,
    );
    if (x, y) != (old.x, old.y) {
        doc.scroll_to(scroll, x, y, blitz_dom::ScrollBehavior::Instant);
    }
}
