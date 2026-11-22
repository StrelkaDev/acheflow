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
pub(crate) fn AddSourceOptions(
    project: Signal<Workspace>,
    server_connected: bool,
    srt_available: bool,
    checking_srt: bool,
    on_refresh_ffmpeg: EventHandler<()>,
    camera_choices: Element,
    microphone_choices: Element,
    on_rtmp: EventHandler<()>,
    on_srt: EventHandler<()>,
    on_added: EventHandler<()>,
) -> Element {
    let control = use_context::<ControlClient>();
    let snapshot = control.state.read();
    let source_types = snapshot
        .as_ref()
        .map(|s| s.source_types.clone())
        .unwrap_or_default();
    rsx! {
        div{id:"source-types",class:"source-types",role:"group","aria-label":"Source type",
            button{disabled:!server_connected,onclick:move |_|on_rtmp.call(()),strong{"RTMP"}span{"Incoming stream with a stream key"}}
            button{id:"add-srt-source",disabled:!server_connected||!srt_available,onclick:move |_|on_srt.call(()),strong{"SRT"}
                span {if !server_connected {"Connect to a server"} else if checking_srt {"Checking FFmpeg…"} else if !srt_available {"Needs an SRT-capable FFmpeg"} else {"Incoming MPEG-TS over UDP"}}
            }
            {camera_choices}
            {microphone_choices}
            button{id:"add-chat-source",onclick:move |_|{add(project,SourceKind::Chat(ChatSource::default()),"Chat".into());on_added.call(());},onkeydown:move |event|{if activates(&event){event.prevent_default();add(project,SourceKind::Chat(ChatSource::default()),"Chat".into());on_added.call(());}},strong{"Chat"}span{"Chat overlay"}}
            for definition in source_types {
                {
                    let name=definition.source_type.name.clone();
                    let kind=SourceKind::Extension(ExtensionSource{extension_id:definition.extension_id.clone(),type_id:definition.source_type.id.clone(),..Default::default()});
                    let label = name.clone();
                    rsx!{button{onclick:move |_|{add(project,kind.clone(),name.clone());on_added.call(());},strong{"{label}"}span{"Extension source"}}}
                }
            }
        }
        if server_connected && !srt_available && !checking_srt {
            div {class:"source-types-footer",button {onclick:move |_|on_refresh_ffmpeg.call(()),"Refresh FFmpeg"}}
        }
    }
}
#[component]
pub(crate) fn VisualSources(mut project: Signal<Workspace>) -> Element {
    let mut expanded = use_signal(std::collections::HashSet::<u32>::new);
    let control = use_context::<ControlClient>();
    let snapshot = control.state.read();
    rsx! {
        for source in project.read().sources.iter().filter(|s|!s.kind.is_media_input()).cloned().collect::<Vec<_>>() {
            section{key:"visual-{source.id}",class:"panel",
                div{class:"panel-heading",button{id:"toggle-visual-{source.id}",class:"destination-toggle",onkeydown:move |event|{if activates(&event){event.prevent_default();let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}}},"aria-expanded":expanded.read().contains(&source.id).to_string(),onclick:move |_|{let mut open=expanded.write();if !open.remove(&source.id){open.insert(source.id);}},span{if expanded.read().contains(&source.id){"▾"}else{"▸"}}h2{"{source.name}"}}span{class:"badge",if matches!(source.kind,SourceKind::Chat(_)){"Chat overlay"}else{"Extension source"}}button{onclick:move |_|project.write().remove_source(source.id),"Remove"}}
                div{style:if expanded.read().contains(&source.id){"display:block"}else{"display:none"},
                div{class:"form-grid",
                    Field{label:"Name",value:source.name.clone(),change:move|v|edit(project,source.id,|s|s.name=v)}
                    NumberField{label:"Initial layer width",value:source.resolution.width,min:16,max:1920,step:2,change:move|v|edit(project,source.id,|s|s.resolution.width=v)}
                    NumberField{label:"Initial layer height",value:source.resolution.height,min:16,max:1080,step:2,change:move|v|edit(project,source.id,|s|s.resolution.height=v)}
                }
                match source.kind {
                    SourceKind::Chat(_)=>rsx!{
                        ChatSettings{project,source_id:source.id}
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
                    SourceKind::Rtmp | SourceKind::Srt { .. } | SourceKind::Camera { .. } | SourceKind::Microphone { .. }=>rsx!{}
                }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ChatPreview(pub Signal<std::collections::HashSet<u32>>);

#[component]
pub(crate) fn ChatSettings(project: Signal<Workspace>, source_id: u32) -> Element {
    let control = use_context::<ControlClient>();
    let local_preview = use_signal(std::collections::HashSet::<u32>::new);
    let ChatPreview(mut preview) =
        try_consume_context::<ChatPreview>().unwrap_or(ChatPreview(local_preview));
    let workspace = project.read();
    let Some(source) = workspace.sources.iter().find(|s| s.id == source_id) else {
        return rsx! {};
    };
    let SourceKind::Chat(settings) = source.kind.clone() else {
        return rsx! {};
    };
    let channels = control
        .state
        .read()
        .as_ref()
        .map(|s| s.channels.clone())
        .unwrap_or_default();
    rsx! {                         div{class:"padded",
        p{class:"form-help","Chat settings · {source.name}. Changes affect every scene layer using this source. Layer CSS controls placement; chat CSS styles messages."}
        button{class:if preview.read().contains(&source_id){"primary"}else{""},"aria-pressed":preview.read().contains(&source_id).to_string(),onclick:move |_|{let mut selected=preview.write();if !selected.remove(&source_id){selected.insert(source_id);}},if preview.read().contains(&source_id){"Hide test messages"}else{"Show test messages"}}
        p{class:"form-help","Test messages appear only in scene previews. They are never sent to channels or live output."}
        div{class:"form-grid",
            NumberField{label:"Maximum messages",value:settings.max_messages,min:1,max:50,step:1,change:move|v|edit_chat(project,source_id,|s|s.max_messages=v)}
            NumberField{label:"Display time (ms)",value:settings.lifetime_ms as u32,min:1000,max:3600000,step:1000,change:move|v|edit_chat(project,source_id,|s|s.lifetime_ms=u64::from(v))}
            NumberField{label:"Appear duration (ms)",value:settings.enter_ms as u32,min:0,max:10000,step:50,change:move|v|edit_chat(project,source_id,|s|s.enter_ms=u64::from(v))}
            NumberField{label:"Disappear duration (ms)",value:settings.exit_ms as u32,min:0,max:10000,step:50,change:move|v|edit_chat(project,source_id,|s|s.exit_ms=u64::from(v))}
        }
        div{class:"row",span{"Channels"}button{class:if settings.channels.is_empty(){"primary"}else{""},"aria-pressed":settings.channels.is_empty().to_string(),onclick:move |_|edit_chat(project,source_id,|s|s.channels.clear()),"All channels"}
            for channel in channels.clone() {
                {let key=channel.key();let selected=settings.channels.contains(&key);rsx!{button{"aria-pressed":selected.to_string(),class:if selected{"primary"}else{""},onclick:move |_|edit_chat(project,source_id,|s|{if s.channels.contains(&key){s.channels.retain(|k|k!=&key)}else{s.channels.push(key.clone());}}),"{channel.label}"}}}
            }
        }
        p{class:"form-help","Selectors: .chat, .chat-message, .chat-author, .chat-body, .chat-platform, .entering, .leaving, .broadcaster, .moderator. Use --chat-enter-duration and --chat-exit-duration with your keyframes."}
        CodeEditor{label:"Chat source CSS",kind:CodeKind::Css,value:settings.css,change:move|v|edit_chat(project,source_id,|s|s.css=v)}
    } }
}

/// Local preview data never enters workspace storage or the control API.
pub(crate) fn test_document(
    settings: &ChatSource,
    now: u64,
) -> acheflow_media_api::visual::SourceDocument {
    use acheflow_extension_api::interaction::{ChatEntry, ChatMessage};
    let entries = [
        ("Viewer", "Hello! Checking chat layout 👋", ""),
        ("Moderator", "Длинное тестовое сообщение для проверки переноса текста и размера карточки. This also tests wrapping across multiple lines.", "moderator"),
        ("Host", "Welcome to the stream!\nSecond line · Привет ✨", "broadcaster"),
        ("Another viewer", "A_long_unbroken_message_to_check_overflow_1234567890_abcdefghijklmnopqrstuvwxyz", ""),
    ].into_iter().enumerate().map(|(index, (author, text, role))| ChatEntry {
        cursor: index as u64,
        message: ChatMessage {
            platform: "Preview".into(), channel_id: "preview".into(), message_id: index.to_string(), user_id: index.to_string(), display_name: author.into(), text: text.into(), roles: if role.is_empty(){vec![]}else{vec![role.into()]}, occurred_at_ms: now,
        },
        deleted: false, merged_into: None, delivery: vec![],
    }).collect::<Vec<_>>();
    let mut config = settings.clone();
    config.channels.clear();
    config.document(&entries, now)
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
        srt_available: bool,
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
            request_generation: use_signal(|| 0),
        };
        use_context_provider(|| client);
        rsx! {
            AddSourceOptions{project,server_connected:true,srt_available:harness.srt_available,checking_srt:false,on_refresh_ffmpeg:|_|{},camera_choices:rsx!{},microphone_choices:rsx!{},on_rtmp:|_|{},on_srt:|_|{},on_added:|_|{}}
            VisualSources{project}
        }
    }
    #[tokio::test]
    async fn srt_add_control_explains_availability_and_blocks_unsupported_servers() {
        for supported in [false, true] {
            let harness = Rc::new(Harness {
                srt_available: supported,
                ..Default::default()
            });
            let mut doc = dioxus_native::DioxusDocument::new(
                VirtualDom::new(app).with_root_context(harness),
                DocumentConfig::default(),
            );
            doc.initial_build();
            settle(&mut doc);
            assert_eq!(
                doc.inner
                    .borrow()
                    .query_selector("#add-srt-source:not([disabled])")
                    .unwrap()
                    .is_some(),
                supported
            );
            assert!(
                doc.inner
                    .borrow()
                    .query_selector("#add-srt-source")
                    .unwrap()
                    .is_some()
            );
            assert!(
                doc.inner
                    .borrow()
                    .query_selector("#add-chat-source")
                    .unwrap()
                    .is_some()
            );
        }
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
    #[test]
    fn test_messages_ignore_channel_filters_without_changing_saved_settings() {
        let settings = ChatSource {
            channels: vec!["example:channel".into()],
            max_messages: 3,
            ..Default::default()
        };
        let original = settings.clone();
        let doc = test_document(&settings, 1000);
        doc.validate().unwrap();
        assert_eq!(doc.nodes[0].children.len(), 3);
        assert!(
            doc.nodes[0]
                .children
                .iter()
                .any(|n| n.classes.contains("moderator"))
        );
        assert!(
            doc.nodes[0]
                .children
                .iter()
                .any(|n| n.classes.contains("broadcaster"))
        );
        assert_eq!(settings, original);
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
        doc.handle_ui_event(UiEvent::KeyDown(event.clone()));
        settle(&mut doc);
        let project = harness.project.get().unwrap();
        assert_eq!(project.peek().sources.len(), 1);
        let html = doc.inner.borrow().root_element().outer_html();
        assert!(html.contains("aria-expanded=\"false\""));
        let toggle = doc
            .inner
            .borrow()
            .query_selector("#toggle-visual-1")
            .unwrap()
            .unwrap();
        doc.inner.borrow_mut().set_focus_to(toggle);
        doc.handle_ui_event(UiEvent::KeyDown(event.clone()));
        settle(&mut doc);
        assert!(
            doc.inner
                .borrow()
                .root_element()
                .outer_html()
                .contains("aria-expanded=\"true\"")
        );
        doc.handle_ui_event(UiEvent::KeyDown(event));
        settle(&mut doc);
        assert!(
            doc.inner
                .borrow()
                .root_element()
                .outer_html()
                .contains("aria-expanded=\"false\"")
        );
        assert!(html.contains("Show test messages"));
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
