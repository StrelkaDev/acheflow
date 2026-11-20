//! Explicit cleanup of server copies retained for migration; never downloads tokens.
use super::*;
use crate::{server_access, server_settings::ActionButton};
use acheflow_media_api::personal_account::*;
#[component]
pub(crate) fn LegacyAccounts() -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let mut expanded = use_signal(|| false);
    let mut accounts = use_signal(Vec::<PersonalAccount>::new);
    let mut message = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut revision = use_signal(|| 0u64);
    let identity = use_memo(move || {
        (
            client.project.read().id.clone(),
            client.project.read().server_url.clone(),
            server_access::identity(),
        )
    });
    use_resource(move || {
        let owner = identity();
        let load = expanded();
        let _ = revision();
        async move {
            if !load {
                return;
            }
            let result = async {
                let base = crate::media::server_url(&client.project.peek(), "/api/access")?;
                server_access::get::<PersonalAccounts>(&format!(
                    "{base}/personal-accounts/{}",
                    owner.0
                ))
                .await
            }
            .await;
            if identity() != owner {
                return;
            }
            match result {
                Ok(data) => accounts.set(data.accounts),
                Err(error) => message.set(error),
            }
        }
    });
    rsx! {section{class:"panel legacy-accounts",
        ActionButton{expanded:Some(expanded()),onclick:move|_|expanded.set(!expanded()),"Clean up old account copies"}
        if expanded(){div{class:"padded",
            p{class:"form-help","Reauthorize in Client on each device before deleting an old server copy. Personal sending now uses only your device account."}
            if accounts.read().is_empty(){p{class:"form-help","No saved personal accounts."}}
            for account in accounts(){div{class:"row",span{"{account.provider} · {account.account_id}"}
                ActionButton{disabled:busy(),onclick:move|_|{let owner=identity();let account=account.clone();busy.set(true);spawn(async move{
                    let result=async{
                        let url=crate::media::server_url(&client.project.peek(),&format!("/api/access/personal-accounts/{}/{}",owner.0,account.provider))?;
                        server_access::decode::<PersonalAccountDisconnected>(server_access::request(reqwest::Method::DELETE,&url)?.json(&DisconnectPersonalAccount{expected_version:account.version}).timeout(std::time::Duration::from_secs(15)).send().await.map_err(|_|"Cannot remove old server copy.")?).await
                    }.await;
                    if identity()!=owner{return;}
                    match result{Ok(result)=>{message.set(if result.credentials_removed{"Old server copy removed."}else{"Account disconnected. Its old credential slot could not be removed; unlock the server credential store before cleanup."}.into());revision+=1;},Err(error)=>message.set(error)}
                    busy.set(false);
                });},"Delete old server copy"}
            }}
            if !message().is_empty(){p{role:"status",class:"form-help","{message}"}}
        }}
    }}
}
