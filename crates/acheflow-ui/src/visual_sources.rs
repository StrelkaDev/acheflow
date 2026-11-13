use crate::{
    code_editor::{CodeEditor, CodeKind},
    control_client::ControlClient,
    live::{Field, NumberField},
    project::Workspace,
};
use acheflow_media_api::visual::{ChatSource, ExtensionSource, SourceKind};
use dioxus_native::prelude::*;
fn edit(
    mut project: Signal<Workspace>,
    id: u32,
    change: impl FnOnce(&mut acheflow_media_api::workspace::Source),
) {
    if let Some(source) = project.write().sources.iter_mut().find(|s| s.id == id) {
        change(source);
    }
}
fn edit_chat(project: Signal<Workspace>, id: u32, change: impl FnOnce(&mut ChatSource)) {
    edit(project, id, |source| {
        if let SourceKind::Chat(settings) = &mut source.kind {
            change(settings)
        }
    });
}
fn add(mut project: Signal<Workspace>, kind: SourceKind, name: String) {
    if project.peek().sources.len() >= 8 {
        return;
    }
    let mut workspace = project.write();
    workspace.add_source();
    if let Some(source) = workspace.sources.last_mut() {
        source.name = name;
        source.kind = kind;
        source.url.clear();
        source.key.clear();
        source.resolution.width = 420;
        source.resolution.height = 720;
    }
}
#[component]
pub(crate) fn VisualSources(mut project: Signal<Workspace>) -> Element {
    let control = use_context::<ControlClient>();
    let snapshot = control.state.read();
    let source_types = snapshot
        .as_ref()
        .map(|s| s.source_types.clone())
        .unwrap_or_default();
    let channels = snapshot
        .as_ref()
        .map(|s| s.channels.clone())
        .unwrap_or_default();
    let full = project.read().sources.len() >= 8;
    rsx! {
        div{class:"row source-actions",
            button{id:"add-chat-source",class:"primary",disabled:full,onclick:move |_|add(project,SourceKind::Chat(ChatSource::default()),"Chat".into()),onkeydown:move |event|{if activates(&event){event.prevent_default();add(project,SourceKind::Chat(ChatSource::default()),"Chat".into());}},"+ Chat source"}
            for definition in source_types {
                {
                    let label=format!("+ {}",definition.source_type.name);
                    let name=definition.source_type.name.clone();
                    let kind=SourceKind::Extension(ExtensionSource{extension_id:definition.extension_id.clone(),type_id:definition.source_type.id.clone(),..Default::default()});
                    rsx!{button{disabled:full,onclick:move |_|add(project,kind.clone(),name.clone()),"{label}"}}
                }
            }
        }
        for source in project.read().sources.iter().filter(|s|!s.kind.is_rtmp()).cloned().collect::<Vec<_>>() {
            section{key:"visual-{source.id}",class:"panel",
                div{class:"panel-heading",h2{"{source.name}"}span{class:"badge",if matches!(source.kind,SourceKind::Chat(_)){"Chat overlay"}else{"Extension source"}}button{onclick:move |_|project.write().remove_source(source.id),"Remove"}}
                div{class:"form-grid",
                    Field{label:"Name",value:source.name.clone(),change:move|v|edit(project,source.id,|s|s.name=v)}
                    NumberField{label:"Initial layer width",value:source.resolution.width,min:16,max:1920,step:2,change:move|v|edit(project,source.id,|s|s.resolution.width=v)}
                    NumberField{label:"Initial layer height",value:source.resolution.height,min:16,max:1080,step:2,change:move|v|edit(project,source.id,|s|s.resolution.height=v)}
                }
                match source.kind {
                    SourceKind::Chat(settings)=>rsx!{
                        div{class:"padded",
                            p{class:"form-help","Add this source to a scene. Layer controls set its position and size; CSS below styles the messages."}
                            div{class:"form-grid",
                                NumberField{label:"Maximum messages",value:settings.max_messages,min:1,max:50,step:1,change:move|v|edit_chat(project,source.id,|s|s.max_messages=v)}
                                NumberField{label:"Display time (ms)",value:settings.lifetime_ms as u32,min:1000,max:3600000,step:1000,change:move|v|edit_chat(project,source.id,|s|s.lifetime_ms=u64::from(v))}
                                NumberField{label:"Appear duration (ms)",value:settings.enter_ms as u32,min:0,max:10000,step:50,change:move|v|edit_chat(project,source.id,|s|s.enter_ms=u64::from(v))}
                                NumberField{label:"Disappear duration (ms)",value:settings.exit_ms as u32,min:0,max:10000,step:50,change:move|v|edit_chat(project,source.id,|s|s.exit_ms=u64::from(v))}
                            }
                            div{class:"row",span{"Channels"}button{class:if settings.channels.is_empty(){"primary"}else{""},"aria-pressed":settings.channels.is_empty().to_string(),onclick:move |_|edit_chat(project,source.id,|s|s.channels.clear()),"All channels"}
                                for channel in channels.clone() {
                                    {let key=channel.key();let selected=settings.channels.contains(&key);rsx!{button{"aria-pressed":selected.to_string(),class:if selected{"primary"}else{""},onclick:move |_|edit_chat(project,source.id,|s|{if s.channels.contains(&key){s.channels.retain(|k|k!=&key)}else{s.channels.push(key.clone());}}),"{channel.label}"}}}
                                }
                            }
                            p{class:"form-help","Selectors: .chat, .chat-message, .chat-author, .chat-body, .chat-platform, .entering, .leaving, .broadcaster, .moderator. Use --chat-enter-duration and --chat-exit-duration with your keyframes."}
                            CodeEditor{label:"Chat source CSS",kind:CodeKind::Css,value:settings.css,change:move|v|edit_chat(project,source.id,|s|s.css=v)}
                        }
                    },
                    SourceKind::Extension(settings)=>{
                        let definition=snapshot.as_ref().and_then(|s|s.source_types.iter().find(|t|t.extension_id==settings.extension_id&&t.source_type.id==settings.type_id)).cloned();
                        rsx!{div{class:"padded",
                            p{class:"form-help","This source receives visual documents from its enabled extension. Apply source changes to make them available to that instance."}
                            if let Some(definition)=definition {
                                for setting in definition.source_type.settings {
                                    {let key=setting.id.clone();let value=settings.settings.get(&key).cloned().unwrap_or_default();rsx!{div{class:"field",span{"{setting.label}"}input{"aria-label":setting.label,value,maxlength:setting.max_length,oninput:move|event|edit(project,source.id,|s|if let SourceKind::Extension(config)=&mut s.kind{config.settings.insert(key.clone(),event.value());})}}}}
                                }
                            }
                            CodeEditor{label:"Source CSS",kind:CodeKind::Css,value:settings.css,change:move|v|edit(project,source.id,|s|if let SourceKind::Extension(config)=&mut s.kind{config.css=v;})}
                        }}
                    },
                    SourceKind::Rtmp=>rsx!{}
                }
            }
        }
    }
}

fn activates(event: &KeyboardEvent) -> bool {
    !event.is_auto_repeating()
        && (event.key() == Key::Named(NamedKey::Enter) || event.key() == Key::Character(" ".into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use blitz_dom::{Document, DocumentConfig};
    use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
    use std::{cell::Cell, rc::Rc};
    #[derive(Default)]
    struct Harness {
        project: Cell<Option<Signal<Workspace>>>,
    }
    fn app() -> Element {
        let harness = use_context::<Rc<Harness>>();
        let project = use_signal(Workspace::default);
        harness.project.set(Some(project));
        let client = ControlClient {
            project,
            state: use_signal(|| None),
            error: use_signal(|| None),
            extension_errors: use_signal(Default::default),
            busy: use_signal(|| false),
            sync_conflict: use_signal(|| false),
        };
        use_context_provider(|| client);
        rsx! {VisualSources{project}}
    }
    fn settle(doc: &mut dioxus_native::DioxusDocument) {
        for _ in 0..40 {
            if !doc.poll(None) {
                doc.inner.borrow_mut().resolve(0.);
                return;
            }
        }
        panic!("visual source UI did not settle");
    }
    #[tokio::test]
    async fn chat_source_is_created_by_keyboard_and_preserves_css_on_roundtrip() {
        let harness = Rc::new(Harness::default());
        let mut doc = dioxus_native::DioxusDocument::new(
            VirtualDom::new(app).with_root_context(harness.clone()),
            DocumentConfig::default(),
        );
        doc.initial_build();
        settle(&mut doc);
        let id = doc
            .inner
            .borrow()
            .query_selector("#add-chat-source")
            .unwrap()
            .unwrap();
        doc.inner.borrow_mut().set_focus_to(id);
        let event = BlitzKeyEvent {
            key: Key::Named(NamedKey::Enter),
            code: Code::Enter,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: None,
        };
        doc.handle_ui_event(UiEvent::KeyDown(event));
        settle(&mut doc);
        let project = harness.project.get().unwrap();
        assert_eq!(project.peek().sources.len(), 1);
        let html = doc.inner.borrow().root_element().outer_html();
        assert!(html.contains("Chat source CSS"));
        assert!(html.contains("Disappear duration"));
        assert!(!html.contains("Connection token"));
        edit_chat(project, 1, |s| s.css = ".chat-author {color:orange}".into());
        let exported = serde_json::to_vec(&*project.peek()).unwrap();
        let restored: Workspace = serde_json::from_slice(&exported).unwrap();
        assert!(
            matches!(&restored.sources[0].kind,SourceKind::Chat(settings) if settings.css.contains("orange"))
        );
    }
}
