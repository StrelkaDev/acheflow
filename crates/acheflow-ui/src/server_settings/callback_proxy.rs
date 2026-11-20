use super::*;
use acheflow_media_api::{
    access::Permission as P, control::ControlAction, workspace::normalize_callback_proxy_url,
};

#[component]
pub(super) fn CallbackProxySettings(mut project: Signal<Workspace>) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let local = use_memo(move || project.read().client_callback_proxy_url.clone());
    let shared = use_memo(move || project.read().callback_proxy_url.clone());
    let mut local_draft = use_signal(move || local.peek().clone());
    let mut server_draft = use_signal(move || shared.peek().clone());
    let mut error = use_signal(String::new);
    use_effect(move || local_draft.set(local()));
    use_effect(move || server_draft.set(shared()));
    let can_configure = crate::server_access::allows(P::ExtensionsConfigure);
    rsx! {section{class:"panel server-settings",
        div{class:"panel-heading",div{h2{"Callback proxy"}p{"Choose the static host that returns callbacks to the waiting Acheflow client."}}}
        div{class:"padded",
            p{class:"form-help","Leave blank to use https://app.acheflow.com. A custom HTTPS origin must host the matching Acheflow callback pages. This changes the callback host, not the workspace server connection."}
            div{class:"server-connection",
                label{class:"field",span{"Client callback proxy URL"}input{r#type:"url","aria-label":"Client callback proxy URL",placeholder:"https://app.acheflow.com",value:local_draft(),oninput:move|event|local_draft.set(event.value())}}
                ActionButton{onclick:move |_|match normalize_callback_proxy_url(&local_draft.peek()){
                    Ok(address)=>{project.write().client_callback_proxy_url=address;error.set(String::new());},
                    Err(reason)=>error.set(reason),
                },"Save on this device"}
            }
            p{class:"form-help","Applies to personal accounts in this device’s workspace. It is not synchronized to other clients."}
            if can_configure {
                div{class:"server-connection",
                    label{class:"field",span{"Server callback proxy URL"}input{r#type:"url","aria-label":"Server callback proxy URL",placeholder:"https://app.acheflow.com",value:server_draft(),disabled:(client.busy)(),oninput:move|event|server_draft.set(event.value())}}
                    ActionButton{disabled:(client.busy)(),onclick:move |_|match normalize_callback_proxy_url(&server_draft.peek()){
                        Ok(url)=>{client.command(ControlAction::SetCallbackProxy{url});error.set(String::new());},
                        Err(reason)=>error.set(reason),
                    },"Save on server"}
                }
                p{class:"form-help","Applies when authorizing the server’s accounts. Requires permission to configure server extensions."}
            }
            if !error().is_empty(){p{class:"field-error",role:"alert","{error}"}}
            if let Some(message)=(client.error)(){p{class:"field-error",role:"alert","{message}"}}
        }
    }}
}
