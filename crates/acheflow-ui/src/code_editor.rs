use acheflow_media_api::appearance::{validate_css, validate_shader};
use dioxus_native::prelude::*;
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
    let mut shown = use_signal(|| value.clone());
    let mut error = use_signal(|| None::<String>);
    use_effect(use_reactive((&value,), move |(value,)| {
        draft.set(value.clone());
        shown.set(value);
        error.set(None);
    }));
    let commit = move || {
        let text = draft.peek().clone();
        match kind.validate(&text) {
            Ok(()) => {
                shown.set(text.clone());
                change.call(text);
                error.set(None);
            }
            Err(e) => error.set(Some(e)),
        }
    };
    rsx! {div {class:"code-editor",
        label {"{label}",textarea {rows:12,spellcheck:"false","aria-label":label,value:shown(),
            oninput:move |e|{draft.set(e.value());if error.peek().is_some(){error.set(None);}},
            onblur:move |_|{let mut commit=commit;commit();},
        }}
        div {class:"row",button {onclick:move |_|{let mut commit=commit;commit();},"Update layer"}
            button {onclick:move |_|{spawn(async move {
                let extension=if kind==CodeKind::Css{"css"}else{"wgsl"};
                if let Some(file)=rfd::AsyncFileDialog::new().add_filter("Layer code",&[extension]).pick_file().await {
                    let bytes=file.read().await;
                    if bytes.len()>32768 {error.set(Some("Code file is too large".into()));return;}
                    match String::from_utf8(bytes).map_err(|e|e.to_string()).and_then(|text|kind.validate(&text).map(|_|text)) {
                        Ok(text)=>{draft.set(text.clone());shown.set(text.clone());change.call(text);error.set(None);},Err(e)=>error.set(Some(e)),
                    }
                }
            });},"Load file"}
            span {class:"form-help","Update applies to the editor; Apply scene & sources updates the server."}
        }
        if let Some(error)=error.read().as_ref(){pre {class:"code-error",role:"alert","{error}"}}
    }}
}
