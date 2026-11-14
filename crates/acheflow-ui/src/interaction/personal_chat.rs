use super::*;
use crate::server_access;
use acheflow_extension_api::{client_execution::ExecutionOutcome, integration::Channel};
use acheflow_media_api::personal_account::*;
#[component]
pub(super) fn Composer(channels: Vec<Channel>, mut text: Signal<String>) -> Element {
    let client = use_context::<ControlClient>();
    let initial = if channels.len() == 1 {
        channels[0].key()
    } else {
        String::new()
    };
    let mut target = use_signal(|| initial);
    let mut accounts = use_signal(|| None::<PersonalAccounts>);
    let mut last = use_context::<ChatDraft>().personal_pending;
    let mut busy = use_signal(|| false);
    let mut status = use_signal(String::new);
    let scope = use_memo(move || {
        let p = client.project.read();
        (
            p.id.clone(),
            p.server_url.clone(),
            server_access::identity(),
        )
    });
    use_resource(move || {
        let actor = scope();
        let endpoint =
            crate::media::server_url(&client.project.peek(), "/api/access/personal-accounts");
        accounts.set(None);
        status.set(String::new());
        busy.set(false);
        async move {
            let Ok(endpoint) = endpoint else {
                return;
            };
            if actor.2.is_none() {
                return;
            }
            loop {
                let result =
                    server_access::get::<PersonalAccounts>(&format!("{endpoint}/{}", actor.0))
                        .await;
                if *scope.peek() != actor {
                    return;
                }
                if let Ok(value) = result {
                    accounts.set(Some(value));
                }
                crate::scene_preview::pause(5000).await;
            }
        }
    });
    let choice = target();
    let selected = channels.iter().find(|c| c.key() == choice).cloned();
    let account = selected.as_ref().and_then(|c| {
        accounts.read().as_ref().and_then(|a| {
            a.accounts
                .iter()
                .find(|a| a.provider == c.extension_id)
                .cloned()
        })
    });
    let can_send = account.is_some()
        && crate::server_access::allows(acheflow_media_api::access::Permission::ChatSend)
        && accounts
            .read()
            .as_ref()
            .is_some_and(|a| !a.send_epoch.is_empty());
    let mut send = move |_: ()| {
        if *busy.peek() {
            return;
        }
        let checking = last.peek().is_some();
        let op = last.peek().clone().or_else(|| {
            let account = account.as_ref()?;
            let data = accounts.peek();
            let data = data.as_ref()?;
            if !can_send || text.peek().trim().is_empty() {
                return None;
            }
            Some((
                choice.clone(),
                SendPersonalChat {
                    request_id: crate::project::new_token(),
                    epoch: data.send_epoch.clone(),
                    issued_at_ms: data.server_time_ms,
                    expected_version: account.version.clone(),
                    text: text.peek().clone(),
                },
            ))
        });
        let Some((channel, request)) = op else {
            return;
        };
        let Some((provider, channel_id)) = channel.split_once(':') else {
            return;
        };
        let actor = scope();
        let endpoint = crate::media::server_url(
            &client.project.peek(),
            &format!(
                "/api/access/personal-send/{}/{provider}/{channel_id}",
                actor.0
            ),
        );
        last.set(Some((channel, request.clone())));
        busy.set(true);
        status.set(String::new());
        spawn(async move {
            let result = async {
                let endpoint = endpoint.map_err(|e| (false, e))?;
                let response = server_access::request(reqwest::Method::POST, &endpoint)
                    .map_err(|e| (false, e))?
                    .json(&request)
                    .timeout(std::time::Duration::from_secs(35))
                    .send()
                    .await
                    .map_err(|_| {
                        (
                            true,
                            "Delivery is uncertain. Check delivery before resending.".to_owned(),
                        )
                    })?;
                let uncertain = response.status().is_server_error();
                server_access::decode::<PersonalDelivery>(response)
                    .await
                    .map_err(|e| (uncertain, e))
            }
            .await;
            if *scope.peek() != actor {
                return;
            }
            match result {
                Ok(delivery) => {
                    match delivery.outcome {
                        ExecutionOutcome::Sent => {
                            text.set(String::new());
                            last.set(None);
                            status.set("Sent from your personal account.".into());
                        }
                        ExecutionOutcome::Unknown => status
                            .set("Delivery is uncertain. Check delivery before resending.".into()),
                        ExecutionOutcome::Failed | ExecutionOutcome::Rejected => {
                            last.set(None);
                            status.set("Not sent. Check your account authorization and channel permissions.".into());
                        }
                    }
                }
                Err((uncertain, error)) => {
                    if !uncertain && !checking {
                        last.set(None);
                    }
                    status.set(error);
                }
            }
            busy.set(false);
        });
    };
    rsx! {div{class:"chat-compose personal-chat-compose",
        label{class:"field",span{"Channel"}select{value:target(),disabled:busy()||last.read().is_some(),onchange:move|e|target.set(e.value()),option{value:"","Choose one channel"}for channel in channels{option{value:channel.key(),"{channel.label}"}}}}
        div{class:"chat-compose-input",input{"aria-label":"Personal message",placeholder:"Write as yourself…",maxlength:8192,value:text(),disabled:busy()||last.read().is_some()||!can_send,oninput:move|e|text.set(e.value())}
            ActionButton{class:"primary",disabled:busy()||(last.read().is_none()&&(!can_send||text.read().trim().is_empty())),onclick:move |_|send(()),if last.read().is_some(){"Check delivery"}else{"Send as me"}}
        }
        if !can_send{p{class:"form-help","Choose a channel and save your personal account in Extensions. Your device also needs Send chat permission."}}
        if last.read().is_some(){ActionButton{onclick:move|_|{last.set(None);text.set(String::new());status.set(String::new());},disabled:busy(),"New message"}}
        if !status.read().is_empty(){p{class:"form-help",role:"status","{status}"}}
    }}
}

#[cfg(test)]
mod tests;
