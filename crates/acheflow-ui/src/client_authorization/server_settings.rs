//! A separate, ephemeral authorizer for the server part. No device vault access.
use super::*;
use crate::{control_client::ControlClient, server_access, server_settings::ActionButton};
use acheflow_extension_api::{integration::GuestManifest, parts::ExecutionHost};
use acheflow_media_api::access::Permission as P;

#[component]
pub(crate) fn ServerSettings() -> Element {
    let client = use_context::<ControlClient>();
    let identity = use_memo(move || {
        (
            client.project.read().id.clone(),
            client.project.read().server_url.clone(),
            server_access::identity(),
            server_access::allows(P::ExtensionsConfigure),
        )
    });
    let catalog = use_resource(move || {
        let owner = identity();
        async move {
            if !owner.3 {
                return Ok(Vec::new());
            }
            let url = crate::media::server_url(
                &client.project.peek(),
                "/api/extensions/server-providers",
            )?;
            let result = server_access::get::<Vec<GuestManifest>>(&url).await?;
            if identity() != owner {
                return Err("Server access changed.".to_owned());
            }
            result
                .into_iter()
                .map(|m| {
                    serde_json::to_string(&m).map_err(|_| "Invalid server manifest.".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        }
    });
    if !server_access::allows(P::ExtensionsConfigure) {
        return rsx! {p{class:"form-help","Your role can view shared chat. Server integration settings require extension configuration permission."}};
    }
    match &*catalog.read_unchecked() {
        Some(Ok(manifests)) => {
            rsx! {for source in manifests {ServerCard{key:"{identity():?}:{source}",source:source.clone()}}}
        }
        Some(Err(error)) => rsx! {p{class:"field-error","{error}"}},
        None => rsx! {p{class:"form-help","Loading server integrations…"}},
    }
}
#[component]
fn ServerCard(source: String) -> Element {
    let manifest: GuestManifest = match serde_json::from_str(&source) {
        Ok(m) => m,
        Err(_) => return rsx! {},
    };
    let mut expanded = use_signal(|| false);
    let client = use_context::<ControlClient>();
    let current = client
        .project
        .read()
        .server_integrations
        .get(&manifest.id)
        .cloned();
    let state = client
        .state
        .read()
        .as_ref()
        .and_then(|s| {
            s.server_providers
                .iter()
                .find(|p| p.provider == manifest.id)
        })
        .map(|p| p.state.clone())
        .unwrap_or_else(|| "Not configured".into());
    let label = match state.as_str() {
        "running" => "Connected",
        "needs_authorization" => "Sign-in required",
        "storage_unavailable" => "Credential storage unavailable",
        "not_authorized" => "Channel permission required",
        "reconnecting" => "Reconnecting",
        "failed" => "Unavailable",
        "stopped" if current.is_some() => "Stopped",
        "connecting" => "Connecting",
        _ => "Not configured",
    };
    rsx! {section{class:"panel extension-card server-integration",
        div{class:"panel-heading",
            div{class:"extension-card-heading",
                div{class:"extension-identity",h2{"{manifest.name}"}span{class:if state=="running"{"badge account-connected"}else{"badge"},"{label}"}}
                if let Some(binding)=current {p{class:"form-help","{binding.account_id} · Channel {binding.channel_id}"}}
                else{p{class:"form-help","Connect the server account to collect shared chat."}}
            }
            ActionButton{expanded:Some(expanded()),onclick:move|_|expanded.set(!expanded()),if expanded(){"Close settings"}else{"Account & settings"}}
        }
        if state=="storage_unavailable" {p{class:"padded form-help",role:"alert","The saved server account could not be opened. Unlock the server’s credential storage, then open Account & settings and choose Save settings to retry. A new sign-in is not required for a temporary storage error."}}
        if expanded(){div{class:"padded extension-settings",ServerLoad{source}}}
    }}
}
#[component]
fn ServerLoad(source: String) -> Element {
    let client = use_context::<ControlClient>();
    let installed = use_context::<Rc<ClientCatalog>>();
    let source_for_load = source.clone();
    let module = use_resource(move || {
        let source = source_for_load.clone();
        let can_run = server_access::allows(P::ExtensionsRun);
        async move {
            let manifest: GuestManifest =
                serde_json::from_str(&source).map_err(|_| "Invalid server manifest.")?;
            let settings_url = crate::media::server_url(
                &client.project.peek(),
                &format!("/api/access/server-accounts/{}", manifest.id),
            )?;
            let settings = server_access::get::<
                acheflow_media_api::server_provider::ServerIntegrationSettings,
            >(&settings_url)
            .await?;
            if settings.workspace_id.as_deref() != Some(client.project.peek().id.as_str()) {
                return Err("Open the shared server workspace first.".to_owned());
            }
            let settings =
                serde_json::to_string(&settings).map_err(|_| "Invalid server settings.")?;
            if !can_run {
                return Ok((Vec::new(), settings));
            }
            let url = crate::media::server_url(
                &client.project.peek(),
                &format!("/api/extensions/server-providers/{}/module", manifest.id),
            )?;
            server_account::require_secure(&url)?;
            let response = server_access::request(reqwest::Method::GET, &url)?
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|_| "Cannot load server authorization.")?;
            if !response.status().is_success() {
                return Err("Authorization code is unavailable. Check execution permission and package approval.".to_owned());
            }
            if response
                .content_length()
                .is_some_and(|n| n > 4 * 1024 * 1024)
            {
                return Err("Server module exceeds limit.".into());
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|_| "Cannot read server authorization.")?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err("Server module exceeds limit.".into());
            }
            Ok((bytes.to_vec(), settings))
        }
    });
    let adapter = serde_json::from_str::<GuestManifest>(&source)
        .ok()
        .and_then(|m| installed.installed.get(&m.id))
        .and_then(|i| i.browser_auth.clone());
    match &*module.read_unchecked() {
        Some(Ok((module, settings))) => {
            rsx! {ServerAuthorization{source,module:module.clone(),adapter,settings_source:settings.clone()}}
        }
        Some(Err(error)) => rsx! {p{class:"field-error","{error}"}},
        None => rsx! {p{class:"form-help","Loading authorization…"}},
    }
}
#[component]
fn ServerAuthorization(
    source: String,
    module: Vec<u8>,
    adapter: Option<String>,
    settings_source: String,
) -> Element {
    let original = use_context::<ControlClient>();
    let mut manifest: GuestManifest = serde_json::from_str(&source).unwrap();
    manifest = match manifest.select_part(ExecutionHost::Server) {
        Ok(Some(m)) => m,
        _ => return rsx! {p{class:"field-error","Invalid server extension part."}},
    };
    let id = manifest.id.clone();
    let name = manifest.name.clone();
    let mut project = use_signal(|| original.project.peek().clone());
    let callback_proxy = use_memo(move || original.project.read().callback_proxy_url.clone());
    use_effect(move || project.write().callback_proxy_url = callback_proxy());
    let sessions = use_signal(Sessions::new);
    use_context_provider(|| sessions);
    let saved = try_consume_context::<ServerDrafts>();
    let draft_key = (
        original.project.peek().id.clone(),
        original.project.peek().server_url.clone(),
        id.clone(),
    );
    let read_key = draft_key.clone();
    let save_id = id.clone();
    use_drop(move || {
        if let Some(saved) = saved {
            let mut drafts = saved.0;
            let config = project
                .peek()
                .client_extensions
                .get(&save_id)
                .map(|s| s.config.clone());
            if let Some(config) = config {
                drafts.write().insert(draft_key, config);
            }
        }
    });
    use_hook(|| {
        let settings: acheflow_media_api::server_provider::ServerIntegrationSettings =
            serde_json::from_str(&settings_source).unwrap();
        let binding = settings.binding;
        if let Some(binding) = binding.clone() {
            project
                .write()
                .server_integrations
                .insert(id.clone(), binding);
        } else {
            project.write().server_integrations.remove(&id);
        }
        let entry = crate::project::ExtensionSettings {
            enabled: true,
            config: manifest.settings_config(
                &saved
                    .and_then(|s| s.0.peek().get(&read_key).cloned())
                    .unwrap_or_else(|| binding.map(|b| b.config).unwrap_or_default()),
            ),
        };
        project.write().client_extensions.insert(id.clone(), entry);
    });
    let client = ControlClient {
        project,
        ..original
    };
    use_context_provider(|| client);
    use_context_provider(|| {
        let mut catalog = ClientCatalog::default();
        catalog.installed.insert(
            id.clone(),
            acheflow_client::catalog::Installed {
                manifest: manifest.clone(),
                module,
                browser_auth: adapter,
            },
        );
        Rc::new(catalog)
    });
    let mut change_account = use_signal(|| false);
    let bound = project.read().server_integrations.contains_key(&id);
    let authorize = server_access::allows(P::ExtensionsRun) && (!bound || change_account());
    rsx! {div{class:"server-authorization",
        div{class:"extension-settings-group",h3{"Server features"}
            div{class:"extension-functions",for setting in manifest.settings.iter().filter(|s|!s.secret&&s.toggle).cloned(){crate::interaction::ExtensionSetting{extension:id.clone(),setting,local:true}}}
            div{class:"extension-fields",for setting in manifest.settings.iter().filter(|s|!s.secret&&!s.toggle&&!manifest.application_setup.as_ref().is_some_and(|setup|setup.settings.contains(&s.id))).cloned(){crate::interaction::ExtensionSetting{extension:id.clone(),setting,local:true}}}
        }
        if authorize {ClientAuthorization{id,name,server:true}}
        else{
            server_account::ServerAccount{id:id.clone(),offer_authorization:false}
            if server_access::allows(P::ExtensionsRun){ActionButton{class:"access-text-button",onclick:move |_|change_account.set(true),"Change server account"}}
            else{p{class:"form-help","Execution permission is required to authorize an account or start/stop collection."}}
        }
    }}
}
