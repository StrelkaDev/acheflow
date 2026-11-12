use crate::{
    control_client::ControlClient,
    live_client::{Action, LiveClient},
};
use acheflow_client::catalog::ClientCatalog;
use acheflow_extension_api::interaction::{CommandRule, PollRequest};
use acheflow_media_api::control::ControlAction;
use dioxus_native::{NodeHandle, prelude::*};
use std::rc::Rc;

#[component]
pub fn Notice() -> Element {
    let client = use_context::<ControlClient>();
    let live = use_context::<LiveClient>();
    rsx! {
        if let Some(error)=client.error.read().as_ref(){p{class:"field-error",role:"alert","{error}"}}
        if (client.sync_conflict)() {
            div{class:"panel-note row",
                button{disabled:(client.busy)() || (live.busy)(),onclick:move |_|client.resolve_sync_conflict(true),"Use server settings"}
                button{disabled:(client.busy)() || (live.busy)(),onclick:move |_|client.resolve_sync_conflict(false),"Keep local edits"}
            }
        }
        if client.state.read().as_ref().is_none_or(|s|s.workspace_id.as_deref()!=Some(client.project.read().id.as_str())) {
            div{class:"panel-note row",p{"Apply this workspace to connect its chat and extensions."}button{disabled:(live.busy)(),onclick:move |_|live.command(Action::Apply),"Apply workspace"}}
        }
    }
}
#[component]
pub fn Audience() -> Element {
    let client = use_context::<ControlClient>();
    let state = client
        .state
        .read()
        .clone()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()));
    let audience = state
        .as_ref()
        .map(|s| s.audience.clone())
        .unwrap_or_default();
    let total = audience
        .total
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into());
    let peak = audience
        .peak
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into());
    let history = state
        .as_ref()
        .map(|s| s.history.iter().rev().take(120).rev().collect::<Vec<_>>())
        .unwrap_or_default();
    let maximum = history
        .iter()
        .filter_map(|p| p.total)
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let points = history
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            p.total.map(|n| {
                format!(
                    "{:.1},{:.1}",
                    i as f64 * 600. / history.len().saturating_sub(1).max(1) as f64,
                    100. - n as f64 / maximum * 95.
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    rsx! {section{class:"panel audience-panel",
        div{class:"panel-heading",h2{"Audience"}span{class:"badge",if audience.partial{"Partial data"}else{"Current session"}}}
        div{class:"stats padded",div{class:"stat",span{class:"stat-label","CURRENT VIEWERS"}strong{"{total}"}p{"Sum across platforms"}}div{class:"stat",span{class:"stat-label","SESSION PEAK"}strong{"{peak}"}p{"Simultaneous sampled audience"}}}
        if !points.is_empty(){svg{class:"audience-chart",view_box:"0 0 600 110",role:"img","aria-label":"Current session viewer history",polyline{points:"{points}",fill:"none",stroke:"#ff975c",stroke_width:"2"}}}
        if audience.channels.is_empty(){p{class:"padded form-help","Connect a platform to see its audience."}}
        for channel in audience.channels {
            div{class:"audience-row",strong{"{channel.sample.platform} / {channel.sample.channel_id}"}span{if let Some(count)=channel.current{"{count} viewers"}else if channel.stale{"Data is stale"}else{"Unavailable"}}
                if let Some(unique)=channel.unique {p{class:"form-help","Unique viewers: {unique.value} · {unique.scope} · ","{date(unique.period_start_ms)} – {date(unique.period_end_ms)}",if unique.provisional{" · Provisional"}}}
            }
        }
    }}
}
fn date(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms.min(i64::MAX as u64) as i64)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "Unknown period".into())
}

#[derive(Clone, Copy)]
struct ChatDraft {
    text: Signal<String>,
    targets: Signal<Vec<String>>,
}

pub fn use_chat_draft(project: Signal<crate::project::Workspace>) {
    let mut text = use_signal(String::new);
    let mut targets = use_signal(Vec::new);
    use_context_provider(|| ChatDraft { text, targets });
    let identity = use_memo(move || {
        let workspace = project.read();
        (workspace.id.clone(), workspace.server_url.clone())
    });
    use_effect(move || {
        let _ = identity();
        text.set(String::new());
        targets.set(Vec::new());
    });
}

#[component]
fn ChatIcon(path: String) -> Element {
    rsx! {svg{class:"chat-icon",view_box:"0 0 24 24","aria-hidden":"true","focusable":"false",path{d:path,fill:"currentColor",fill_rule:"evenodd"}}}
}
fn chat_brand(catalog: &ClientCatalog, id: &str) -> (String, String) {
    let installed = catalog.installed.get(id);
    let path = installed
        .and_then(|i| i.manifest.chat_presentation.as_ref())
        .map(|p| p.icon_path.clone())
        .unwrap_or_else(|| "M2 3h20v14H9l-5 4v-4H2Zm2 2v10h2v2l3-2h11V5Z".into());
    let name = installed
        .map(|i| i.manifest.name.clone())
        .unwrap_or_else(|| id.into());
    (path, name)
}
fn chat_channel_help(
    catalog: &ClientCatalog,
    channel: &acheflow_extension_api::integration::Channel,
) -> String {
    let access = if channel.capabilities.iter().any(|c| c == "chat.send") {
        "Receive and send"
    } else {
        "Read only"
    };
    let help = catalog
        .installed
        .get(&channel.extension_id)
        .and_then(|i| i.manifest.chat_presentation.as_ref())
        .map(|p| p.help.as_str())
        .unwrap_or_default();
    format!("{} · {} · {access}. {help}", channel.label, channel.id)
}

#[component]
pub fn ChatPanel() -> Element {
    let client = use_context::<ControlClient>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let ChatDraft {
        mut text,
        mut targets,
    } = use_context::<ChatDraft>();
    let state = client
        .state
        .read()
        .clone()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .unwrap_or_default();
    let channels = state
        .channels
        .iter()
        .filter(|c| {
            c.capabilities
                .iter()
                .any(|s| s == "chat.read" || s == "chat.send")
        })
        .cloned()
        .collect::<Vec<_>>();
    let selected = targets();
    // Keep explicit selections when a channel disconnects. Never silently fan out
    // a draft intended for that channel to every remaining provider.
    let can_send = channels.iter().any(|c| {
        (selected.is_empty() || selected.contains(&c.key()))
            && c.capabilities.iter().any(|s| s == "chat.send")
    });
    rsx! {
        section{class:"panel chat-panel",
            div{class:"panel-heading",h2{"Chat"}if channels.is_empty(){span{class:"form-help","No connected channel"}}}
            Notice{}
            div{class:"chat-history",role:"log","aria-label":"Chat messages",
                if state.chat.entries.is_empty(){p{class:"form-help","Messages will appear when a connected channel receives chat."}}
                for entry in state.chat.entries.clone() {
                    {
                        let (icon, name) = chat_brand(&catalog, &entry.message.platform);
                        let help = channels.iter().find(|c|c.extension_id==entry.message.platform && c.id==entry.message.channel_id)
                            .map(|c|chat_channel_help(&catalog,c)).unwrap_or_else(||format!("{name} · {}",entry.message.channel_id));
                        rsx!{div{key:"{entry.cursor}",class:"chat-message",
                            span{class:"chat-origin chat-tooltip",tabindex:"0","aria-label":help.clone(),title:help.clone(),ChatIcon{path:icon}span{class:"chat-tooltip-text",role:"tooltip","{help}"}}
                            strong{"{entry.message.display_name}"}
                            span{class:"chat-body",if entry.deleted{"Message deleted"}else{"{entry.message.text}"}}
                            for delivery in entry.delivery {
                                {
                                    let provider = delivery.channel.split(':').next().unwrap_or_default();
                                    let (icon, name) = chat_brand(&catalog, provider);
                                    let help = format!("{name} · {}: {}{}", delivery.channel, delivery.state, delivery.error.as_ref().map(|e|format!(" · {e}")).unwrap_or_default());
                                    rsx!{span{class:"chat-delivery chat-tooltip",tabindex:"0",title:help.clone(),"aria-label":help.clone(),ChatIcon{path:icon}span{class:"chat-delivery-state","{delivery_symbol(&delivery.state)}"}span{class:"chat-tooltip-text",role:"tooltip","{help}"}}}
                                }
                            }
                        }}
                    }
                }
            }
            div{class:"chat-compose",
                div{class:"chat-compose-input",input{"aria-label":"Message",placeholder:if can_send{"Write a message…"}else{"No selected channel can send"},disabled:!can_send,maxlength:8192,value:"{text}",oninput:move|e|text.set(e.value())}
                    button{class:"primary",disabled:(client.busy)() || !can_send || text().trim().is_empty(),onclick:move |_|{client.command(ControlAction::SendChat{targets:targets(),text:text()});text.set(String::new());},"Send"}
                }
                div{class:"chat-targets",role:"group","aria-label":"Send message to",
                    button{class:if selected.is_empty(){"chat-target selected"}else{"chat-target"},"aria-pressed":selected.is_empty().to_string(),title:"Send to all available channels that support sending",onclick:move |_|targets.set(Vec::new()),onkeydown:move |event|{if chat_activation(&event){event.prevent_default();targets.set(Vec::new());}},"Send all"}
                    for channel in channels {
                        {
                            let key = channel.key();
                            let keyboard_key = key.clone();
                            let active = selected.contains(&key);
                            let sendable = channel.capabilities.iter().any(|c|c=="chat.send");
                            let help = chat_channel_help(&catalog, &channel);
                            let (icon, _) = chat_brand(&catalog, &channel.extension_id);
                            rsx!{span{class:"chat-tooltip",tabindex:if sendable{"-1"}else{"0"},
                                button{class:if active{"chat-target selected"}else{"chat-target"},disabled:!sendable,"aria-label":help.clone(),"aria-pressed":active.to_string(),title:help.clone(),onclick:move |_|{
                                    toggle_chat_target(targets, &key);
                                },onkeydown:move |event|{if sendable && chat_activation(&event){event.prevent_default();toggle_chat_target(targets,&keyboard_key);}},ChatIcon{path:icon}if active{span{class:"chat-target-check","✓"}}}
                                span{class:"chat-tooltip-text",role:"tooltip","{help}"}
                            }}
                        }
                    }
                }
            }
        }
    }
}
fn chat_activation(event: &KeyboardEvent) -> bool {
    event.key() == Key::Named(NamedKey::Enter) || event.key() == Key::Character(" ".into())
}
fn toggle_chat_target(mut targets: Signal<Vec<String>>, key: &str) {
    let mut next = targets();
    if next.iter().any(|id| id == key) {
        next.retain(|id| id != key);
    } else {
        next.push(key.into());
    }
    targets.set(next);
}
fn delivery_symbol(state: &str) -> &'static str {
    match state {
        "sent" => "✓",
        "failed" => "!",
        "pending" | "queued" => "…",
        _ => "?",
    }
}

#[component]
pub fn InteractivePage() -> Element {
    let client = use_context::<ControlClient>();
    let mut channel = use_signal(String::new);
    let mut question = use_signal(String::new);
    let mut options = use_signal(|| "Yes; No".to_owned());
    let state = client
        .state
        .read()
        .clone()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .unwrap_or_default();
    let channels = state
        .channels
        .iter()
        .filter(|c| c.capabilities.iter().any(|s| s == "poll.create"))
        .map(|c| c.key())
        .collect::<Vec<_>>();
    let target = if channels.contains(&channel()) {
        channel()
    } else {
        String::new()
    };
    let poll_target = target.clone();
    rsx! {
        div{class:"page-heading",div{h1{"Interactive"}p{"Polls and viewer interactions with your scenes."}}}
        Notice{}
        div{class:"interaction-channel",
            crate::live::Choice{label:"Channel",value:target.clone(),choices:std::iter::once("All channels".to_owned()).chain(channels.iter().map(|key|state.channels.iter().find(|c|c.key()==*key).map(|c|c.label.clone()).unwrap_or_else(||key.clone()))).collect::<Vec<_>>(),values:Some(std::iter::once(String::new()).chain(channels.iter().cloned()).collect::<Vec<_>>()),change:move |id|channel.set(id)}
            if channels.is_empty(){p{class:"form-help","Connect a channel in Extensions to start polls and configure commands."}}
        }
        div{class:"interaction-grid",
                section{class:"panel",div{class:"panel-heading",h2{"Polls"}}
                    div{class:"padded",label{class:"field",span{"Question"}input{value:"{question}",maxlength:500,oninput:move|e|question.set(e.value())}}label{class:"field",span{"Options, separated by ;"}input{value:"{options}",oninput:move|e|options.set(e.value())}}button{disabled:(client.busy)() || channels.is_empty() || question().trim().is_empty(),onclick:move |_|client.command(ControlAction::CreatePoll{poll:PollRequest{question:question(),options:options().split(';').map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect::<Vec<_>>(),channels:if poll_target.is_empty(){vec![]}else{vec![poll_target.clone()]},duration_seconds:60}}),"Start 60-second poll"}p{class:"form-help","Results are updated by the selected extensions."}}
                    for poll in state.polls.iter().rev().take(5).cloned(){div{class:"poll-result",strong{"{poll.question}"}for (index,option) in poll.options.iter().enumerate(){p{"{index+1}. {option} — {poll.votes[index]}"}}for target in poll.targets.clone() {p{class:"form-help","{target.channel}: {target.state}"}if let Some(error)=target.error{p{class:"field-error","{error}"}}}if !poll.closed && poll.targets.iter().any(|t| t.state!="closing"){div{class:"row",button{onclick:move |_|client.command(ControlAction::ClosePoll{id:poll.id,cancel:false}),"Close"}button{onclick:move |_|client.command(ControlAction::ClosePoll{id:poll.id,cancel:true}),"Cancel"}}}else{p{class:"form-help",if !poll.closed{"Collecting final results…"}else if poll.cancelled{"Cancelled"}else{"Closed"}}}}}
                }
            CommandEditor{channel:state.channels.iter().find(|c|c.capabilities.iter().any(|s|s=="chat.send")).map(|c|c.key()).unwrap_or_default()}
            section{class:"panel",div{class:"panel-heading",h2{"Donations"}}div{class:"padded",if state.donations.is_empty(){p{class:"form-help","Incoming donations appear here."}}for donation in state.donations.iter().rev(){div{class:"row",strong{"{donation.display_name}"}span{"{money(donation.amount_minor,donation.decimal_places)} {donation.currency}"}span{"{donation.message}"}}}}}
        }
    }
}

#[component]
fn CommandEditor(channel: String) -> Element {
    let client = use_context::<ControlClient>();
    let mut name = use_signal(|| "spin".to_owned());
    let mut scene = use_signal(|| 0u32);
    let mut object = use_signal(|| 0u64);
    let mut role = use_signal(|| "moderator".to_owned());
    let state = client
        .state
        .read()
        .clone()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .unwrap_or_default();
    let mut interaction_id = use_signal(String::new);
    let selected_interaction = state
        .interactions
        .iter()
        .find(|i| i.id == interaction_id())
        .cloned();
    let interaction_key = selected_interaction
        .as_ref()
        .map(|i| i.id.clone())
        .unwrap_or_default();
    let interaction_label = selected_interaction
        .as_ref()
        .map(|i| i.label.clone())
        .unwrap_or_else(&*name);
    let scenes = client.project.read().scenes.clone();
    let scene_id = if scene() == 0 {
        scenes.first().map(|s| s.id).unwrap_or(0)
    } else {
        scene()
    };
    let objects = scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.spatial.as_ref())
        .map(|s| s.objects.clone())
        .unwrap_or_default();
    let object_id = if object() == 0 {
        objects.first().map(|o| o.id).unwrap_or(0)
    } else {
        object()
    };
    rsx! {section{class:"panel",div{class:"panel-heading",h2{"Chat commands"}}div{class:"padded",
        label{class:"field",span{"Command (without !)"}input{value:"{name}",maxlength:32,oninput:move|e|name.set(e.value())}}
        crate::live::Choice{label:"Scene",value:scene_id.to_string(),choices:scenes.iter().map(|s|s.name.clone()).collect::<Vec<_>>(),values:Some(scenes.iter().map(|s|s.id.to_string()).collect::<Vec<_>>()),change:move |id:String|{scene.set(id.parse().unwrap_or(0));object.set(0);}}
        crate::live::Choice{label:"Object",value:object_id.to_string(),choices:objects.iter().map(|o|o.name.clone()).collect::<Vec<_>>(),values:Some(objects.iter().map(|o|o.id.to_string()).collect::<Vec<_>>()),change:move |id:String|object.set(id.parse().unwrap_or(0))}
        crate::live::Choice{label:"Who may use it",value:role(),choices:vec!["Moderators".into(),"Broadcaster".into(),"Everyone".into()],values:Some(vec!["moderator".into(),"broadcaster".into(),String::new()]),change:move |value|role.set(value)}
        button{disabled:channel.is_empty() || object_id==0 || (client.busy)(),onclick:move |_|client.command(ControlAction::SetCommand{command:CommandRule{name:name(),channel:channel.clone(),roles:if role().is_empty(){vec![]}else{vec![role()]},cooldown_ms:5000,scene_id,object_id,rotation:[0.,45.,0.],enabled:true}}),"Bind to 45° Y rotation"}
        crate::live::Choice{label:"Interaction",value:interaction_key.clone(),choices:std::iter::once("New interaction".into()).chain(state.interactions.iter().map(|i|i.label.clone())).collect::<Vec<_>>(),values:Some(std::iter::once(String::new()).chain(state.interactions.iter().map(|i|i.id.clone())).collect::<Vec<_>>()),change:move|id|interaction_id.set(id)}
        button{disabled:object_id==0 || (client.busy)(),onclick:move |_|client.command(ControlAction::SetInteraction{interaction:acheflow_extension_api::integration::Interaction {id:if interaction_key.is_empty(){name()}else{interaction_key.clone()},label:interaction_label.clone(),action:Some(acheflow_extension_api::integration::SceneAction {scene_id,object_id,rotation:[0.,45.,0.]}),enabled:true}}),"Save interaction binding"}
        for interaction in state.interactions.clone(){div{class:"row",span{"{interaction.label}"}button{disabled:!interaction.enabled || interaction.id.split_once(':').is_some_and(|(origin,_)| origin!="acheflow" && !state.extensions.iter().any(|e| e.id==origin && !matches!(e.state.as_str(),"stopped"|"failed"))) || (client.busy)(),onclick:move |_|client.command(ControlAction::TriggerInteraction{id:interaction.id.clone()}),"Trigger"}}}
        p{class:"form-help","Five-second cooldown. Select a 3D object to enable the binding."}
        for rule in state.commands.clone(){div{class:"row command-row",span{"!{rule.name}"}button{onclick:{let mut rule=rule.clone();rule.enabled = !rule.enabled;move |_|client.command(ControlAction::SetCommand{command:rule.clone()})},if rule.enabled{"Disable"}else{"Enable"}}}}
    }}}
}

#[component]
pub fn ExtensionsPage() -> Element {
    let client = use_context::<ControlClient>();
    let live = use_context::<LiveClient>();
    let workspace = client.project.read().clone();
    let snapshot = client.state.read().clone().unwrap_or_default();
    let extensions = snapshot.extensions.clone();
    let local = use_context::<Rc<ClientCatalog>>();
    let scope = use_signal(|| {
        if local.installed.is_empty() && local.errors.is_empty() {
            1
        } else {
            0
        }
    });
    let mut handles: Signal<[Option<NodeHandle>; 2]> = use_signal(|| [None, None]);
    let client_label = if cfg!(target_arch = "wasm32") {
        "Browser client"
    } else {
        "Desktop client"
    };
    let server = crate::media::server_url(&workspace, "").unwrap_or_else(|e| e);
    rsx! {
        div {class:"page-heading",div {h1 {"Extensions"}p {"Client integrations and server logic for this workspace."}}}
        nav{class:"subnav extension-scope-tabs",role:"tablist","aria-label":"Extension scope",
            for (index,label) in [(0,"Client"),(1,"Server")] {
                button{class:if scope()==index{"selected"}else{""},role:"tab",id:if index==0{"extensions-tab-client"}else{"extensions-tab-server"},
                    "aria-selected":(scope()==index).to_string(),"aria-controls":if index==0{"extensions-panel-client"}else{"extensions-panel-server"},tabindex:if scope()==index{0}else{-1},
                    onmounted:move |e|handles.write()[index]=e.data().downcast::<NodeHandle>().cloned(),
                    onclick:move |_|select_extension_scope(scope,handles,index),
                    onkeydown:move |event|{
                        let next=match event.key(){
                            Key::Named(NamedKey::ArrowLeft|NamedKey::ArrowRight)=>1-index,
                            Key::Named(NamedKey::Home)=>0,
                            Key::Named(NamedKey::End)=>1,
                            Key::Named(NamedKey::Enter)=>index,
                            Key::Character(value) if value==" "=>index,
                            _=>return,
                        };
                        event.prevent_default();select_extension_scope(scope,handles,next);
                    },
                    "{label}"
                }
            }
        }
        section{class:"extension-scope-panel",id:"extensions-panel-client",role:"tabpanel","aria-labelledby":"extensions-tab-client",hidden:scope()!=0,tabindex:0,
            p{class:"form-help extension-scope-context","{client_label} · {workspace.name}"}
            p{class:"form-help","Installed in this client. Accounts and credentials stay on the executor."}
            for error in &local.errors {p{class:"field-error","{error}"}}
            if local.installed.is_empty() {p{class:"form-help","No integrations are installed in this client."}}
            for (id,installed) in &local.installed {
                ClientIntegrationCard {key:"client:{workspace.id}:{id}",id:id.clone(),name:installed.manifest.name.clone()}
            }
        }
        section{class:"extension-scope-panel",id:"extensions-panel-server",role:"tabpanel","aria-labelledby":"extensions-tab-server",hidden:scope()!=1,tabindex:0,
            p{class:"form-help extension-scope-context","Server: {server}"}
            Notice {}
            if client.state.read().is_none(){p{class:"form-help","Connecting to the selected server…"}}
            if client.state.read().is_some() && extensions.is_empty() {p{class:"form-help","No logic extensions are installed on this server."}}
            for status in extensions {
                ExtensionCard {key:"server:{server}:{workspace.id}:{status.id}", id:status.id, name:status.name, busy:(live.busy)()}
            }
            WorkspaceCapabilities {}
        }
    }
}

fn select_extension_scope(
    mut selected: Signal<usize>,
    handles: Signal<[Option<NodeHandle>; 2]>,
    index: usize,
) {
    selected.set(index);
    let handle = handles.peek()[index].clone();
    // Defer focus until event dispatch releases Blitz's document borrow.
    spawn(async move {
        if let Some(handle) = handle {
            handle.doc_mut().set_focus_to(handle.node_id());
        }
    });
}

#[component]
fn ExtensionCard(id: String, name: String, busy: bool) -> Element {
    let client = use_context::<ControlClient>();
    let snapshot = client.state.read().clone().unwrap_or_default();
    let active = snapshot.workspace_id.as_deref() == Some(client.project.read().id.as_str());
    let status = snapshot
        .extensions
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .unwrap_or_default();
    let settings = client
        .project
        .read()
        .extensions
        .get(&id)
        .cloned()
        .unwrap_or_default();
    let enabled = settings.enabled;
    let error = client
        .extension_errors
        .read()
        .get(&id)
        .cloned()
        .or_else(|| {
            if active && enabled {
                status.error.clone()
            } else {
                None
            }
        });
    let body_id = format!("extension-{id}");
    let toggle_id = id.clone();
    let keyboard_id = id.clone();
    let capabilities = status.capabilities.join(", ");
    let state_label = if !enabled {
        None
    } else if !active {
        Some("Apply workspace to start")
    } else {
        match status.state.as_str() {
            "running" => None,
            "waiting_for_client" => Some("Waiting for a client executor"),
            "connecting" => Some("Connecting"),
            "reconnecting" => Some("Reconnecting"),
            "needs_authorization" => Some("Connect an account"),
            "failed" => Some("Failed"),
            _ => Some("Ready to start"),
        }
    };
    let unavailable = busy || (client.busy)();
    rsx! {
        section {class:if enabled {"panel extension-card enabled"} else {"panel extension-card"},
            div {class:"panel-heading",
                div {class:"extension-identity",h2 {"{name}"}if let Some(label) = state_label {span {class:"badge","{label}"}}}
                button {class:if enabled {"extension-toggle selected"} else {"extension-toggle"},
                    role:"switch", "aria-label":"Enable {name} for this workspace", "aria-checked":enabled.to_string(),
                    "aria-expanded":enabled.to_string(), "aria-controls":body_id.clone(), disabled:unavailable,
                    onclick:move |_|client.set_extension(toggle_id.clone(), !enabled),
                    onkeydown:move |event| {
                        if !unavailable && (event.key() == Key::Named(NamedKey::Enter) || event.key() == Key::Character(" ".into())) {
                            event.prevent_default();
                            client.set_extension(keyboard_id.clone(), !enabled);
                        }
                    },
                    if enabled {"Enabled"} else {"Disabled"}
                }
            }
            if enabled {
                div {class:"padded extension-settings",id:body_id.clone(),
                    div{class:"extension-fields",for setting in status.settings.iter().filter(|s|!s.secret && !s.toggle).cloned() {
                        ExtensionSetting {key:"{setting.id}",extension:id.clone(),setting,local:false}
                    }}
                    div{class:"extension-functions",for setting in status.settings.iter().filter(|s|!s.secret && s.toggle).cloned(){ExtensionSetting{key:"{setting.id}",extension:id.clone(),setting,local:false}}}
                    if active {
                        if let Some(account) = status.account {p {"Account: {account}"}}
                        if !capabilities.is_empty() {p {class:"form-help","Capabilities: {capabilities}"}}
                        for panel in snapshot.panels.iter().filter(|p| p.extension_id == id) {
                            {crate::extension_panel::render(panel)}
                        }
                    }
                }
            }
            if let Some(error) = error {div {class:"panel-note",p {class:"field-error",role:"alert","{error}"}}}
        }
    }
}

#[component]
pub(crate) fn ExtensionSetting(
    extension: String,
    setting: acheflow_extension_api::integration::Setting,
    local: bool,
) -> Element {
    let mut client = use_context::<ControlClient>();
    let workspace = client.project.read();
    let settings = if local {
        &workspace.client_extensions
    } else {
        &workspace.extensions
    };
    let value = settings
        .get(&extension)
        .and_then(|s| s.config.get(&setting.id))
        .cloned()
        .unwrap_or_default();
    drop(workspace);
    if setting.toggle {
        let enabled = value != "false";
        return rsx! {div{class:"extension-setting-toggle",button{class:if enabled{"selected"}else{""},role:"switch","aria-checked":enabled.to_string(),onclick:move |_|{
            let mut w=client.project.write();let map=if local{&mut w.client_extensions}else{&mut w.extensions};
            map.entry(extension.clone()).or_default().config.insert(setting.id.clone(),(!enabled).to_string());
        },span{class:"extension-function-mark","aria-hidden":"true",if enabled{"✓"}else{"−"}}"{setting.label}"}}};
    }
    rsx! {label{class:"field",span{"{setting.label}"}input{r#type:"text",value:value.clone(),maxlength:setting.max_length as i64,oninput:move|event|{let mut w=client.project.write();let map=if local{&mut w.client_extensions}else{&mut w.extensions};map.entry(extension.clone()).or_default().config.insert(setting.id.clone(),event.value());}}if !setting.help.is_empty(){p{class:"form-help","{setting.help}"}}}}
}

#[component]
fn ClientIntegrationCard(id: String, name: String) -> Element {
    let mut client = use_context::<ControlClient>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let manifest = &catalog.installed[&id].manifest;
    let enabled = client
        .project
        .read()
        .client_extensions
        .get(&id)
        .is_some_and(|s| s.enabled);
    let connected = client
        .state
        .read()
        .as_ref()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .is_some_and(|s| {
            s.client_executors
                .iter()
                .any(|e| e.connections.iter().any(|c| c.provider == id))
        });
    let toggle_id = id.clone();
    let key_id = id.clone();
    rsx! {
        section{class:if enabled{"panel extension-card enabled client-integration"}else{"panel extension-card client-integration"},
            div{class:"panel-heading",
                div{class:"extension-identity",h2{"{name}"}if enabled{span{class:"badge",if connected{"Executor connected"}else{"Executor not connected"}}}}
                button{class:if enabled{"extension-toggle selected"}else{"extension-toggle"},role:"switch","aria-label":"Enable {name} locally for this workspace","aria-checked":enabled.to_string(),"aria-expanded":enabled.to_string(),"aria-controls":"client-extension-{id}",
                    onclick:move |_|{client.project.write().client_extensions.entry(toggle_id.clone()).or_default().enabled = !enabled;},
                    onkeydown:move |event|{if event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into()){event.prevent_default();client.project.write().client_extensions.entry(key_id.clone()).or_default().enabled = !enabled;}},
                    if enabled{"Enabled"}else{"Disabled"}
                }
            }
            if enabled{div{class:"padded extension-settings",id:"client-extension-{id}",
                div{class:"extension-fields",for setting in manifest.settings.iter().filter(|s|!s.secret && !s.toggle && !manifest.application_setup.as_ref().is_some_and(|setup|setup.settings.contains(&s.id))).cloned(){ExtensionSetting{key:"{setting.id}",extension:id.clone(),setting,local:true}}}
                div{class:"extension-functions",for setting in manifest.settings.iter().filter(|s|!s.secret && s.toggle).cloned(){ExtensionSetting{key:"{setting.id}",extension:id.clone(),setting,local:true}}}
                crate::client_authorization::ClientAuthorization{key:"{client.project.read().id}:{id}",id:id.clone(),name:name.clone()}
                {client_execution_panel(id.clone())}
            }}
        }
    }
}
fn capability_names<'a>(
    values: impl IntoIterator<Item = &'a acheflow_extension_api::client_execution::Capability>,
) -> String {
    values
        .into_iter()
        .map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ")
}
#[component]
fn WorkspaceCapabilities() -> Element {
    use acheflow_extension_api::client_execution::Capability;
    let client = use_context::<ControlClient>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let state = client.state.read();
    let current = state
        .as_ref()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()));
    rsx! {section{class:"panel workspace-capabilities",
        div{class:"panel-heading",h2{"Workspace capabilities"}}
        div{class:"padded",
            if let Some(state)=current {
                h3{"Saved permissions"}
                if state.client_capabilities.is_empty(){p{class:"form-help","No client capabilities are authorized."}}
                for (index, grant) in state.client_capabilities.iter().enumerate() {
                    div{class:"audience-row",strong{"{grant.provider} / {grant.channel_id}"}p{"Account: {grant.account_id}"}p{class:"form-help","{capability_names(&grant.capabilities)}"}
                        if !grant.capabilities.contains(&Capability::ChatSend) && catalog.installed.get(&grant.provider).is_some_and(|i|i.manifest.subscriptions.iter().any(|s|s=="chat.send")) {
                            {
                                let mut grants=state.client_capabilities.clone();
                                grants[index].capabilities.insert(Capability::ChatSend);
                                let keyboard_grants=grants.clone();
                                rsx!{button{class:"allow-chat-send",disabled:(client.busy)(),"aria-label":format!("Allow sending to {} / {}",grant.provider,grant.channel_id),title:"Allow this account to send chat in this workspace. Platform authorization is still required.",onclick:move |_|client.command(ControlAction::SetClientCapabilities{grants:grants.clone()}),onkeydown:move |event|{if !(client.busy)() && chat_activation(&event){event.prevent_default();client.command(ControlAction::SetClientCapabilities{grants:keyboard_grants.clone()});}},"Allow sending"}}
                            }
                        }
                    }
                }
                h3{"Connected executors"}
                if state.client_executors.is_empty(){p{class:"form-help","No executor is connected to this workspace."}}
                for executor in &state.client_executors {for connection in &executor.connections {div{class:"audience-row",strong{"{executor.client_id} · {connection.provider} / {connection.channel_id}"}p{class:"form-help","{capability_names(&connection.capabilities)}"}}}}
            }else{p{class:"form-help","Apply this workspace to inspect its server permissions and connected executors."}}
        }
    }}
}

fn money(amount: u64, places: u8) -> String {
    let scale = 10u64.pow(u32::from(places.min(6)));
    if places == 0 {
        amount.to_string()
    } else {
        format!(
            "{}.{:0width$}",
            amount / scale,
            amount % scale,
            width = usize::from(places.min(6))
        )
    }
}

fn client_execution_panel(id: String) -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rsx! {crate::client_authorization::ClientExecutionPanel {id}}
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        rsx! {p{class:"form-help","Continuous live chat is available in the desktop client."}}
    }
}
