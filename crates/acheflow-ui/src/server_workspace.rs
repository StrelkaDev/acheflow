use crate::project::Workspace;
use acheflow_media_api::server_workspace::{ImportWorkspace, WorkspaceState};
use dioxus_native::prelude::*;

async fn decode(response: reqwest::Response) -> Result<WorkspaceState, String> {
    if !response.status().is_success() {
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(value["error"]
            .as_str()
            .or_else(|| value["error"]["message"].as_str())
            .unwrap_or("Could not access the saved server workspace.")
            .into());
    }
    response
        .json()
        .await
        .map_err(|_| "Invalid server workspace response.".into())
}
pub(crate) async fn fetch(workspace: &Workspace) -> Result<WorkspaceState, String> {
    let url = crate::media::server_url(workspace, "/api/workspace")?;
    decode(
        crate::server_access::request(reqwest::Method::GET, &url)?
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| "Cannot reach the workspace server.")?,
    )
    .await
}
async fn save(workspace: &Workspace, revision: u64) -> Result<WorkspaceState, String> {
    let url = crate::media::server_url(workspace, "/api/workspace/import")?;
    decode(
        crate::server_access::request(reqwest::Method::POST, &url)?
            .json(&ImportWorkspace {
                expected_revision: revision,
                workspace: workspace.server_configuration(),
            })
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| "Cannot save the workspace on this server.")?,
    )
    .await
}
#[derive(Clone)]
struct Pending {
    workspace: Workspace,
    revision: u64,
}
#[component]
pub(crate) fn SavedWorkspace(mut project: Signal<Workspace>, open: Signal<bool>) -> Element {
    let mut state = use_signal(|| None::<WorkspaceState>);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut pending = use_signal(|| None::<Pending>);
    let mut notice = use_signal(String::new);
    let mut refresh = use_signal(|| 0u64);
    let target = use_memo(move || crate::media::server_url(&project.read(), "/api/workspace"));
    use_resource(move || {
        let target = target();
        let opened = open();
        let _ = refresh();
        state.set(None);
        pending.set(None);
        async move {
            if !opened {
                return;
            }
            let workspace = project.peek().clone();
            let result = fetch(&workspace).await;
            if target != crate::media::server_url(&project.peek(), "/api/workspace") {
                return;
            }
            match result {
                Ok(result) => {
                    state.set(Some(result));
                }
                Err(e) => error.set(e),
            }
        }
    });
    let allowed = state.read().is_some()
        && [
            acheflow_media_api::access::Permission::WorkspaceManage,
            acheflow_media_api::access::Permission::ScenesManage,
            acheflow_media_api::access::Permission::MediaManage,
            acheflow_media_api::access::Permission::ExtensionsConfigure,
            acheflow_media_api::access::Permission::ExtensionsRun,
        ]
        .into_iter()
        .all(crate::server_access::allows);
    rsx! {section {class:"panel server-workspace-card",div {class:"panel-heading",div {h3 {"Saved on the server"}p {if let Some(summary)=state.read().as_ref().and_then(|s|s.summary.as_ref()) {"{summary.name}"}else if state.read().is_some(){"No workspace saved yet"}else{"Loading server workspace…"}}}
            if let Some(state)=state.read().as_ref(){span {class:"badge","Revision {state.revision}"}}
        }
        div {class:"padded",
            p {class:"form-help","Save this local workspace on the connected server. Your local copy and account authorizations stay on this device. Nothing starts automatically."}
            if let Some(review)=pending.read().as_ref(){
                p {"Save ‘{review.workspace.name}’ as the server workspace? This replaces its saved configuration."}
                div {class:"row",crate::server_settings::ActionButton {class:"primary",disabled:busy(),onclick:move |_|{
                    let Some(review)=pending.peek().clone()else{return;};let owner=target();busy.set(true);error.set(String::new());
                    spawn(async move {
                        let result=save(&review.workspace,review.revision).await;
                        if owner!=crate::media::server_url(&project.peek(),"/api/workspace"){busy.set(false);return;}
                        match result {
                            Ok(saved)=>{
                                if let Some(sync)=try_consume_context::<crate::workspace_sync::WorkspaceSync>() {sync.applied(&review.workspace,saved.revision);}
                                if project.peek().id==review.workspace.id && acheflow_media_api::server_workspace::fingerprint(&project.peek()).ok()==acheflow_media_api::server_workspace::fingerprint(&review.workspace).ok(){project.write().edit_revision=saved.revision;}
                                state.set(Some(saved));notice.set("Workspace saved on the server. Activate workspace sources when ready.".into());
                            },
                            Err(e)=>{error.set(e);*refresh.write()+=1;}
                        }
                        pending.set(None);busy.set(false);
                    });
                },if busy(){"Saving…"}else{"Save to server"}}
                crate::server_settings::ActionButton {disabled:busy(),onclick:move |_|pending.set(None),"Cancel"}}
            }else {div {class:"row",crate::server_settings::ActionButton {disabled:busy() || !allowed,onclick:move |_|{if let Some(state)=state.peek().as_ref(){pending.set(Some(Pending {workspace:project.peek().clone(),revision:state.revision}));notice.set(String::new());}},"Save this workspace to server"}}}
            if !error.read().is_empty(){p {role:"alert",class:"field-error","{error}"}}
            if !notice.read().is_empty(){p {role:"status",class:"form-help","{notice}"}}
        }
    }}
}
