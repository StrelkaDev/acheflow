//! Device accounts send directly; no personal token is submitted to the server.
use super::*;
use crate::{client_authorization::Sessions, server_access};
use acheflow_client::direct_action::PlatformHttp;
use acheflow_extension_api::{client_execution::ExecutionOutcome, integration::Channel};
use acheflow_media_api::{
    access::Permission as P,
    direct_action::{AuthorizeDirectAction, DirectActionPermission},
};
#[derive(Clone, PartialEq)]
pub(crate) struct PendingSend {
    id: String,
    channel: String,
    text: String,
    account: String,
    running: bool,
    outcome: ExecutionOutcome,
    error: Option<String>,
}
fn actor(
    client: ControlClient,
    access: Option<server_access::AccessState>,
) -> (String, String, Option<String>) {
    let p = client.project.read();
    (
        p.id.clone(),
        p.server_url.clone(),
        access.and_then(|state| {
            state
                .session
                .peek()
                .as_ref()
                .and_then(|s| serde_json::to_string(s).ok())
        }),
    )
}
#[component]
pub(super) fn Composer(channels: Vec<Channel>, mut text: Signal<String>) -> Element {
    let client = use_context::<ControlClient>();
    let access = try_consume_context::<server_access::AccessState>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let sessions = use_context::<Signal<Sessions>>();
    let ChatDraft {
        mut targets,
        personal_pending: mut pending,
        ..
    } = use_context::<ChatDraft>();
    let one = channels.len() == 1;
    let initial = channels.first().map(Channel::key);
    use_effect(use_reactive((&one, &initial), move |(one, initial)| {
        if one
            && targets.read().is_empty()
            && pending.read().is_none()
            && text.read().is_empty()
            && let Some(key) = &initial
        {
            targets.set(vec![key.clone()]);
        }
    }));
    let choice = targets.read().first().cloned().unwrap_or_default();
    let channel = channels.iter().find(|c| c.key() == choice).cloned();
    let owner = actor(client, access);
    let account = channel.as_ref().and_then(|channel| {
        sessions
            .read()
            .get(&(
                owner.0.clone(),
                owner.1.clone(),
                channel.extension_id.clone(),
            ))
            .and_then(|s| {
                s.credential_revision()
                    .map(|revision| (s.account_id(), revision))
            })
    });
    let capable = channel.as_ref().is_some_and(|c| {
        catalog
            .installed
            .get(&c.extension_id)
            .is_some_and(|i| i.manifest.allows_action("chat.send"))
    });
    let can_send = server_access::allows(P::ChatSend)
        && capable
        && account.is_some()
        && pending.read().is_none();
    let running = pending.read().as_ref().is_some_and(|p| p.running);
    let sender = account
        .as_ref()
        .map(|a| a.0.clone())
        .unwrap_or_else(|| "Not connected on this device".into());
    let send_catalog = catalog.clone();
    rsx! {div{class:"chat-compose personal-chat-compose",
        div{class:"chat-sender row",span{"Send from this device:"}strong{"{sender}"}}
        div{class:"chat-targets",role:"group","aria-label":"Send message to",
            for channel in &channels {
                {let key=channel.key();let selected=key==choice;let key_press=key.clone();let label=channel.label.clone();
                 rsx!{ActionButton{class:if selected{"chat-target selected"}else{"chat-target"},pressed:Some(selected),aria_label:Some(label.clone()),disabled:running,onclick:move|_|targets.set(vec![key_press.clone()]),"{label}"}}}
            }
        }
        if channel.is_none() && !choice.is_empty(){p{class:"field-error","The selected channel disconnected. Choose a connected channel; the draft will not be redirected automatically."}}
        if account.is_none(){p{class:"form-help","Connect your account in Extensions → Client on this device. The server account is independent."}}
        if !server_access::allows(P::ChatSend){p{class:"form-help","This device has read-only chat access."}}
        div{class:"chat-compose-input",input{"aria-label":"Message",placeholder:"Write a message…",disabled:!can_send,maxlength:8192,value:"{text}",oninput:move|e|text.set(e.value())}
            ActionButton{id:"personal-chat-send",class:"primary",disabled:!can_send||text().trim().is_empty(),onclick:move|_|{
                if pending.peek().is_some()||!server_access::allows(P::ChatSend){return;}
                let Some(channel)=channel.clone() else{return;};let Some((account,revision))=account.clone() else{return;};
                let Some(installed)=send_catalog.installed.get(&channel.extension_id) else{return;};
                let current=actor(client,access);let key=(current.0.clone(),current.1.clone(),channel.extension_id.clone());
                let prepared=sessions.peek().get(&key).ok_or_else(||"Reconnect this device's account.".to_owned()).and_then(|session|session.direct_action(&installed.module,&channel.id)).and_then(|action|action.with_stream(channel.stream_id.as_deref()));
                let request=PendingSend{id:crate::project::new_token(),channel:channel.key(),text:text.peek().clone(),account:account.clone(),running:true,outcome:ExecutionOutcome::Unknown,error:None};
                pending.set(Some(request.clone()));
                dioxus_core::spawn_forever(async move{
                    let result=async{
                        authorize(client,access,&current,&channel).await?;
                        let mut action=prepared?;
                        action.prepare(&mut PlatformHttp).await?;
                        action.send(&request.id,&request.text,&mut PlatformHttp,||async{
                            if actor(client,access)!=current{return Err("Workspace or signed-in device changed.".into());}
                            let unchanged=sessions.peek().get(&key).is_some_and(|s|s.account_id()==account&&s.credential_revision()==Some(revision));
                            if !unchanged{return Err("The device account changed before sending.".into());}
                            authorize(client,access,&current,&channel).await
                        }).await
                    }.await;
                    if actor(client,access)!=current||!pending.peek().as_ref().is_some_and(|p|p.id==request.id){return;}
                    match result {
                        Ok(ExecutionOutcome::Sent)=>{text.set(String::new());pending.set(None);},
                        Ok(outcome)=>{let mut next=request;next.running=false;next.outcome=outcome;pending.set(Some(next));},
                        Err(error)=>{let mut next=request;next.running=false;next.outcome=ExecutionOutcome::Rejected;next.error=Some(error);pending.set(Some(next));},
                    }
                });
            },"Send"}
        }
        if let Some(last)=pending(){
            p{class:if last.error.is_some(){"field-error"}else{"form-help"},role:"status",
                if last.running{"Checking this device's account and sending…"}
                else if last.outcome==ExecutionOutcome::Unknown{"Delivery is uncertain. Check the platform before sending again; this request will not be repeated."}
                else if let Some(ref message)=last.error{"{message}"}
                else{"Not sent. Check this account's platform permissions and reconnect if necessary."}
            }
            ActionButton{id:"personal-chat-new",disabled:last.running,onclick:move|_|pending.set(None),"New message"}
        }
    }}
}
async fn authorize(
    client: ControlClient,
    access: Option<server_access::AccessState>,
    owner: &(String, String, Option<String>),
    channel: &Channel,
) -> Result<(), String> {
    if &actor(client, access) != owner
        || !access.is_some_and(|a| {
            a.session
                .peek()
                .as_ref()
                .is_some_and(|s| s.device.permissions.contains(&P::ChatSend))
        })
    {
        return Err("This device is no longer allowed to send here.".into());
    }
    let endpoint =
        crate::media::server_url(&client.project.peek(), "/api/access/actions/authorize")?;
    let started = web_time::Instant::now();
    let response = server_access::request(reqwest::Method::POST, &endpoint)?
        .json(&AuthorizeDirectAction {
            workspace_id: owner.0.clone(),
            provider: channel.extension_id.clone(),
            channel_id: channel.id.clone(),
            stream_id: channel.stream_id.clone(),
            operation: "chat.send".into(),
        })
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "The server could not confirm sending permission.")?;
    let permission = server_access::decode::<DirectActionPermission>(response).await?;
    let lifetime = permission
        .valid_until_ms
        .saturating_sub(permission.server_time_ms)
        .min(5000);
    let authenticated = access.and_then(|state| state.session.peek().clone());
    if !authenticated.is_some_and(|s| {
        s.server_id == permission.server_id
            && s.user.id == permission.user_id
            && s.device.id == permission.device_id
    }) || &actor(client, access) != owner
        || permission.workspace_id != owner.0
        || permission.provider != channel.extension_id
        || permission.channel_id != channel.id
        || permission.stream_id != channel.stream_id
        || started.elapsed().as_millis() >= u128::from(lifetime)
    {
        return Err("Sending permission changed or expired before dispatch.".into());
    }
    Ok(())
}
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
