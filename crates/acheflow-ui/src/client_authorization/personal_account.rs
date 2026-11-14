//! Personal credentials belong to the signed-in profile, not to a shared workspace role.
use super::*;
use crate::{server_access, server_settings::ActionButton};
use acheflow_media_api::personal_account::*;

#[component]
pub(super) fn PersonalAccountPanel(id: String) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let sessions = use_context::<Signal<Sessions>>();
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(String::new);
    let mut expanded = use_signal(|| false);
    let mut trusted = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut saved = use_signal(|| None::<PersonalAccount>);
    let mut notice = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut generation = use_signal(|| 0u64);
    let scope = use_memo({
        let id = id.clone();
        move || {
            let project = client.project.read();
            (
                project.id.clone(),
                project.server_url.clone(),
                project
                    .client_extensions
                    .get(&id)
                    .map(|s| s.config.clone())
                    .unwrap_or_default(),
                server_access::identity(),
            )
        }
    });
    use_effect(move || {
        let _ = scope();
        loaded.set(false);
        load_error.set(String::new());
        expanded.set(false);
        trusted.set(false);
        busy.set(false);
        saved.set(None);
        notice.set(String::new());
        error.set(String::new());
        generation += 1;
    });
    use_resource({
        let id = id.clone();
        move || {
            let actor = scope();
            let id = id.clone();
            let endpoint =
                crate::media::server_url(&client.project.peek(), "/api/access/personal-accounts");
            async move {
                if actor.3.is_none() {
                    return;
                }
                let Ok(endpoint) = endpoint else {
                    return;
                };
                loop {
                    let version = *generation.peek();
                    let result =
                        server_access::get::<PersonalAccounts>(&format!("{endpoint}/{}", actor.0))
                            .await;
                    if *scope.peek() != actor {
                        return;
                    }
                    if (*generation.peek() == version || !*loaded.peek()) && !*busy.peek() {
                        match result {
                            Ok(result) => {
                                saved.set(result.accounts.into_iter().find(|a| a.provider == id));
                                loaded.set(true);
                                load_error.set(String::new());
                            }
                            Err(e) => load_error.set(e),
                        }
                    }
                    crate::scene_preview::pause(5000).await;
                }
            }
        }
    });
    let actor = scope();
    let local = sessions
        .read()
        .get(&(actor.0.clone(), actor.1.clone(), id.clone()))
        .and_then(|s| s.credential_revision().map(|_| s.account_id()));
    let connect = {
        let id = id.clone();
        move |_: ()| {
            if *busy.peek() || !*trusted.peek() || !*loaded.peek() {
                return;
            }
            let actor = scope();
            let credentials = sessions
                .peek()
                .get(&(actor.0.clone(), actor.1.clone(), id.clone()))
                .ok_or("Authorize a local account first.")
                .and_then(|s| {
                    s.server_credentials(&actor.0)
                        .map_err(|_| "Finish local account authorization first.")
                });
            let credentials = match credentials {
                Ok(value) => value,
                Err(e) => {
                    error.set(e.into());
                    return;
                }
            };
            let expected_version = saved.peek().as_ref().map(|a| a.version.clone());
            let target =
                crate::media::server_url(&client.project.peek(), "/api/access/personal-accounts");
            let id = id.clone();
            busy.set(true);
            notice.set(String::new());
            error.set(String::new());
            generation += 1;
            spawn(async move {
                let result=async{
                if *scope.peek()!=actor {return Err("The profile or server changed. Try again.".into());}
                let target=target?;super::server_account::require_secure(&target)?;
                server_access::decode::<PersonalAccount>(server_access::request(reqwest::Method::POST,&format!("{target}/{}/{id}",actor.0))?
                    .json(&ConnectPersonalAccount {trust_server:true,expected_account:credentials.account_id,expected_version,config:actor.2.clone(),credentials:credentials.secrets})
                    .timeout(std::time::Duration::from_secs(40)).send().await.map_err(|_|"Connection could not finish. Check your saved personal account before retrying.")?).await
            }.await;
                if *scope.peek() != actor {
                    return;
                }
                generation += 1;
                match result {
                    Ok(account) => {
                        saved.set(Some(account));
                        expanded.set(false);
                        trusted.set(false);
                        notice.set(
                            "Saved for your profile. The shared chat connection is unchanged."
                                .into(),
                        );
                    }
                    Err(e) => error.set(e),
                }
                busy.set(false);
            });
        }
    };
    let disconnect = {
        let id = id.clone();
        move |_: ()| {
            if *busy.peek() {
                return;
            }
            let Some(account) = saved.peek().clone() else {
                return;
            };
            let actor = scope();
            let target =
                crate::media::server_url(&client.project.peek(), "/api/access/personal-accounts");
            let id = id.clone();
            busy.set(true);
            notice.set(String::new());
            error.set(String::new());
            generation += 1;
            spawn(async move {
                let result=async{
                if *scope.peek()!=actor{return Err("The profile or server changed. Try again.".into());}
                let target=target?;super::server_account::require_secure(&target)?;
                server_access::decode::<PersonalAccountDisconnected>(server_access::request(reqwest::Method::DELETE,&format!("{target}/{}/{id}",actor.0))?
                    .json(&DisconnectPersonalAccount{expected_version:account.version})
                    .timeout(std::time::Duration::from_secs(10)).send().await.map_err(|_|"Cannot disconnect your personal account. Refresh its status before retrying.")?).await
            }.await;
                if *scope.peek() != actor {
                    return;
                }
                generation += 1;
                match result {
                    Ok(result) => {
                        saved.set(None);
                        notice.set(if result.credentials_removed{"Personal account disconnected. Your local copy is unchanged."}else{"Personal account disconnected. The server could not delete the stored credentials."}.into());
                    }
                    Err(e) => error.set(e),
                }
                busy.set(false);
            });
        }
    };
    if actor.3.is_none() {
        return rsx! {};
    }
    rsx! {section{class:"server-account-connect personal-account-connect",
        h4{"Your personal server account"}
        p{class:"form-help","Saved for your profile and its linked devices. Sharing this studio does not share your account or start another chat connection."}
        if let Some(account)=saved.read().clone(){div{class:"server-account-current",p{"Personal account: {account.account_id}"}ActionButton{class:"access-text-button",disabled:busy(),onclick:disconnect,"Disconnect my account"}}}
        if !expanded(){
            if local.is_none(){p{class:"form-help","Authorize a local account above to save it for your profile."}}
            ActionButton{id:"personal-account-review",class:"access-text-button",disabled:busy()||!loaded()||local.is_none(),expanded:Some(false),onclick:move|_|expanded.set(true),"Save this account for my profile…"}
        }else{
            p{class:"form-help","The server will recheck and store a copy of your credentials, including refresh tokens and application secrets. Its administrator can access them. Continue only if you trust this server."}
            if let Some(local)=local.as_ref(){p{class:"form-help","Local account: {local}"}}
            ActionButton{id:"personal-account-trust",class:if trusted(){"access-permission selected"}else{"access-permission"},pressed:Some(trusted()),disabled:busy(),onclick:move|_|trusted.set(!trusted()),span{class:"access-check",if trusted(){"✓"}}span{"I trust this server with my personal account."}}
            ActionButton{id:"personal-account-submit",class:"primary",disabled:busy()||!trusted()||local.is_none(),onclick:connect,if busy(){"Checking account…"}else{"Save my account"}}
            ActionButton{class:"access-text-button",disabled:busy(),onclick:move|_|{expanded.set(false);trusted.set(false);},"Cancel"}
        }
        if !loaded() && load_error.read().is_empty(){p{class:"form-help",role:"status","Loading your saved accounts…"}}
        if !load_error.read().is_empty(){p{class:"field-error",role:"alert","{load_error}"}}
        if !notice.read().is_empty(){p{class:"form-help",role:"status","{notice}"}}
        if !error.read().is_empty(){p{class:"field-error",role:"alert","{error}"}}
    }}
}

#[cfg(test)]
mod tests;
