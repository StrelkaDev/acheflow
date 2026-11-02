use crate::{
    control_client::ControlClient,
    live_client::{Action, LiveClient},
};
use acheflow_extension_api::interaction::{CommandRule, PollRequest};
use acheflow_media_api::control::ControlAction;
use dioxus_native::prelude::*;

#[component]
pub fn Notice() -> Element {
    let client = use_context::<ControlClient>();
    let live = use_context::<LiveClient>();
    rsx! {
        if let Some(error)=client.error.read().as_ref(){p{class:"field-error",role:"alert","{error}"}}
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
    channel: Signal<String>,
}

pub fn use_chat_draft(project: Signal<crate::project::Workspace>) {
    let mut text = use_signal(String::new);
    let mut channel = use_signal(String::new);
    use_context_provider(|| ChatDraft { text, channel });
    let identity = use_memo(move || {
        let workspace = project.read();
        (workspace.id.clone(), workspace.server_url.clone())
    });
    use_effect(move || {
        let _ = identity();
        text.set(String::new());
        channel.set(String::new());
    });
}

#[component]
pub fn ChatPanel() -> Element {
    let client = use_context::<ControlClient>();
    let ChatDraft {
        mut text,
        mut channel,
    } = use_context::<ChatDraft>();
    let state = client
        .state
        .read()
        .clone()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .unwrap_or_default();
    let channels = state
        .audience
        .channels
        .iter()
        .filter(|c| c.sample.platform == "twitch")
        .map(|c| c.sample.channel_id.clone())
        .collect::<Vec<_>>();
    let target = if channels.contains(&channel()) {
        channel()
    } else {
        channels.first().cloned().unwrap_or_default()
    };
    let send_target = target.clone();
    rsx! {
            section{class:"panel chat-panel",div{class:"panel-heading",h2{"Chat"}crate::live::Choice{label:"Channel",value:target.clone(),choices:channels.iter().map(|id|format!("Twitch / {id}")).collect::<Vec<_>>(),values:Some(channels.clone()),change:move |id|channel.set(id)}if channels.is_empty(){span{class:"form-help","No connected channel"}}}

                Notice{}
                div{class:"chat-history",role:"log","aria-label":"Chat messages",
                    if state.chat.entries.is_empty(){p{class:"form-help","Messages will appear when a connected channel receives chat."}}
                    for entry in state.chat.entries {div{key:"{entry.cursor}",class:"chat-message",span{class:"chat-origin","{entry.message.platform}"}strong{"{entry.message.display_name}"}span{if entry.deleted{"Message deleted"}else{"{entry.message.text}"}}}}
                }
                div{class:"chat-compose",input{"aria-label":"Message",placeholder:"Write a message…",maxlength:500,value:"{text}",oninput:move|e|text.set(e.value())}button{class:"primary",disabled:(client.busy)() || target.is_empty() || text().trim().is_empty(),onclick:move |_|{client.command(ControlAction::SendChat{channel_id:send_target.clone(),text:text()});text.set(String::new());},"Send"}}
            }
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
        .audience
        .channels
        .iter()
        .filter(|c| c.sample.platform == "twitch")
        .map(|c| c.sample.channel_id.clone())
        .collect::<Vec<_>>();
    let target = if channels.contains(&channel()) {
        channel()
    } else {
        channels.first().cloned().unwrap_or_default()
    };
    let poll_target = target.clone();
    rsx! {
        div{class:"page-heading",div{h1{"Interactive"}p{"Polls and viewer interactions with your scenes."}}}
        Notice{}
        div{class:"interaction-channel",
            crate::live::Choice{label:"Channel",value:target.clone(),choices:channels.iter().map(|id|format!("Twitch / {id}")).collect::<Vec<_>>(),values:Some(channels.clone()),change:move |id|channel.set(id)}
            if channels.is_empty(){p{class:"form-help","Connect a channel in Extensions to start polls and configure commands."}}
        }
        div{class:"interaction-grid",
                section{class:"panel",div{class:"panel-heading",h2{"Polls"}}
                    div{class:"padded",label{class:"field",span{"Question"}input{value:"{question}",maxlength:500,oninput:move|e|question.set(e.value())}}label{class:"field",span{"Options, separated by ;"}input{value:"{options}",oninput:move|e|options.set(e.value())}}button{disabled:(client.busy)() || target.is_empty() || question().trim().is_empty(),onclick:move |_|client.command(ControlAction::CreatePoll{poll:PollRequest{question:question(),options:options().split(';').map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect::<Vec<_>>(),channels:vec![format!("twitch:{poll_target}")],duration_seconds:60}}),"Start 60-second poll"}p{class:"form-help","Viewers vote with !vote 1, !vote 2…"}}
                    for poll in state.polls.iter().rev().take(5).cloned(){div{class:"poll-result",strong{"{poll.question}"}for (index,option) in poll.options.iter().enumerate(){p{"{index+1}. {option} — {poll.votes[index]}"}}if let Some(error)=poll.announcement_error{p{class:"field-error","Announcement: {error}"}}if !poll.closed{div{class:"row",button{onclick:move |_|client.command(ControlAction::ClosePoll{id:poll.id,cancel:false}),"Close"}button{onclick:move |_|client.command(ControlAction::ClosePoll{id:poll.id,cancel:true}),"Cancel"}}}else{p{class:"form-help",if poll.cancelled{"Cancelled"}else{"Closed"}}}}}
                }
            CommandEditor{channel:target.clone()}
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
        button{disabled:channel.is_empty() || object_id==0 || (client.busy)(),onclick:move |_|client.command(ControlAction::SetCommand{command:CommandRule{name:name(),channel:format!("twitch:{channel}"),roles:if role().is_empty(){vec![]}else{vec![role()]},cooldown_ms:5000,scene_id,object_id,rotation:[0.,45.,0.],enabled:true}}),"Bind to 45° Y rotation"}
        p{class:"form-help","Five-second cooldown. Select a 3D object to enable the binding."}
        for rule in client.state.read().as_ref().map(|s|s.commands.clone()).unwrap_or_default(){div{class:"row command-row",span{"!{rule.name}"}button{onclick:{let mut rule=rule.clone();rule.enabled = !rule.enabled;move |_|client.command(ControlAction::SetCommand{command:rule.clone()})},if rule.enabled{"Disable"}else{"Enable"}}}}
    }}}
}

#[component]
pub fn ExtensionsPage() -> Element {
    let client = use_context::<ControlClient>();
    let mut client_id = use_signal(String::new);
    let state = client.state.read().clone().unwrap_or_default();
    rsx! {div{class:"page-heading",div{h1{"Extensions"}p{"Connect platforms and manage their access to your workspace."}}}Notice{}
        section{class:"panel",div{class:"panel-heading",h2{"Twitch"}span{class:"badge",{state.extensions.iter().find(|e|e.id=="twitch").map(|s|s.state.clone()).unwrap_or_else(||"Not connected".into())}}}
            div{class:"padded",label{class:"field",span{"Twitch application client ID"}input{value:"{client_id}",oninput:move|e|client_id.set(e.value()),placeholder:"Public client ID from your Twitch application"}}
                button{class:"primary",disabled:client_id().is_empty() || (client.busy)(),onclick:move |_|client.command(ControlAction::AuthorizeTwitch{client_id:client_id()}),"Connect Twitch"}
                p{class:"form-help","Use a public Twitch application. You'll authorize chat access on Twitch; credentials stay on the server for this session."}
                if let Some(auth)=state.authorization {div{class:"authorization-box",strong{"Enter code: {auth.user_code}"}a{href:"{auth.verification_uri}",target:"_blank",rel:"noopener noreferrer","Authorize on Twitch"}}}
            }
        }
        for status in state.extensions.clone() {{let id=status.id.clone();let capabilities=status.capabilities.join(", ");let running=matches!(status.state.as_str(),"running"|"connecting"|"reconnecting");rsx!{section{class:"panel",div{class:"panel-heading",h2{"{status.name}"}button{disabled:(client.busy)(),onclick:move |_|client.command(ControlAction::SetExtension{id:id.clone(),enabled:!running}),if running{"Stop"}else{"Start"}}}div{class:"padded",p{"{status.state}"}p{class:"form-help","Capabilities: {capabilities}"}if let Some(account)=status.account{p{"Account: {account}"}}if let Some(error)=status.error{p{class:"field-error",role:"alert","{error}"}}}}}}}
        for panel in state.panels {{crate::extension_panel::render(&panel)}}
    }
}
