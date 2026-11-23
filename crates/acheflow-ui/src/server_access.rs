//! Device pairing, permissions and server-scoped authenticated requests.
mod identity;
use crate::project::Workspace;
use crate::server_settings::ActionButton;
use acheflow_media_api::access::{
    Device, Devices, InviteRequest, PairRequest, PairResponse, Permission as P, Session,
};
use dioxus_native::prelude::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(crate) struct AccessState {
    pub session: Signal<Option<Session>>,
    pub error: Signal<String>,
}
#[cfg(target_arch = "wasm32")]
thread_local! {static TOKENS:std::cell::RefCell<std::collections::HashMap<String,String>>=Default::default();}

pub(crate) fn client() -> reqwest::Client {
    #[cfg(not(target_arch = "wasm32"))]
    {
        acheflow_client::access::client().expect("HTTP client")
    }
    #[cfg(target_arch = "wasm32")]
    {
        reqwest::Client::new()
    }
}
pub(crate) fn authorize(
    request: reqwest::RequestBuilder,
    endpoint: &str,
) -> Result<reqwest::RequestBuilder, String> {
    let request = request.header("x-acheflow-request", "1");
    #[cfg(not(target_arch = "wasm32"))]
    {
        acheflow_client::access::authorize(request, endpoint)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let request = request.fetch_credentials_same_origin();
        let base = base(endpoint)?;
        Ok(TOKENS.with(|tokens| match tokens.borrow().get(&base) {
            Some(token) => request.bearer_auth(token),
            None => request,
        }))
    }
}
pub(crate) fn request(
    method: reqwest::Method,
    endpoint: &str,
) -> Result<reqwest::RequestBuilder, String> {
    authorize(client().request(method, endpoint), endpoint)
}
#[cfg(target_arch = "wasm32")]
fn base(endpoint: &str) -> Result<String, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Invalid server address.")?;
    let path = url
        .path()
        .rsplit_once("/api/")
        .map(|(base, _)| base)
        .unwrap_or(url.path())
        .trim_end_matches('/');
    Ok(format!("{}{path}", url.origin().ascii_serialization()))
}
pub(crate) fn provide(project: Signal<Workspace>) {
    let mut state = AccessState {
        session: use_signal(|| None),
        error: use_signal(String::new),
    };
    use_context_provider(|| state);
    let target = use_memo(move || crate::media::server_url(&project.read(), "/api/access/session"));
    use_resource(move || {
        let endpoint = target();
        state.session.set(None);
        async move {
            let Ok(endpoint) = endpoint else {
                return;
            };
            #[cfg(not(target_arch = "wasm32"))]
            let storage_error = {
                let endpoint = endpoint.clone();
                crate::native_credentials::run(move || acheflow_client::access::restore(&endpoint))
                    .await
                    .err()
            };
            #[cfg(target_arch = "wasm32")]
            let storage_error: Option<String> = None;
            loop {
                match get_session(&endpoint).await {
                    Ok(Some(session)) => {
                        if state.session.peek().as_ref() != Some(&session) {
                            state.session.set(Some(session));
                        }
                        state.error.set(String::new());
                    }
                    Ok(None) => {
                        if state.session.peek().is_some() {
                            state.session.set(None);
                        }
                        state.error.set(storage_error.clone().unwrap_or_default());
                    }
                    Err(message) => {
                        state.session.set(None);
                        state.error.set(message);
                    }
                }
                crate::scene_preview::pause(2000).await;
            }
        }
    });
}
pub(crate) fn identity() -> Option<String> {
    try_consume_context::<AccessState>().and_then(|state| {
        state
            .session
            .read()
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok())
    })
}
pub(crate) fn allows(permission: P) -> bool {
    try_consume_context::<AccessState>().is_some_and(|state| {
        state
            .session
            .read()
            .as_ref()
            .is_some_and(|session| session.device.permissions.contains(&permission))
    })
}
pub(crate) async fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();
    if !status.is_success() {
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Server access request failed.")
            .into());
    }
    response
        .json()
        .await
        .map_err(|_| "Invalid server access response.".into())
}
pub(crate) async fn get<T: serde::de::DeserializeOwned>(endpoint: &str) -> Result<T, String> {
    decode(
        request(reqwest::Method::GET, endpoint)?
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| "Cannot reach the server.")?,
    )
    .await
}
async fn get_session(endpoint: &str) -> Result<Option<Session>, String> {
    let response = request(reqwest::Method::GET, endpoint)?
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "Cannot reach the server.")?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    decode(response).await.map(Some)
}
async fn pair_device(endpoint: &str, code: String, name: String) -> Result<Session, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Invalid server address.")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) {
        return Err("Open the server through trusted HTTPS before pairing.".into());
    }
    let paired: PairResponse = decode(
        request(reqwest::Method::POST, endpoint)?
            .header("x-acheflow-client", "native")
            .json(&PairRequest { code, name })
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| "Cannot pair with this server.")?,
    )
    .await?;
    if let Some(token) = paired.token {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let endpoint = endpoint.to_owned();
            crate::native_credentials::run(move || {
                acheflow_client::access::remember(&endpoint, &token)
            })
            .await?;
        }
        #[cfg(target_arch = "wasm32")]
        TOKENS.with(|tokens| {
            tokens
                .borrow_mut()
                .insert(base(endpoint).unwrap_or_default(), token)
        });
    }
    Ok(paired.session)
}

#[derive(Clone, Copy)]
enum Role {
    Viewer,
    Chat,
    Scenes,
    Operator,
    Administrator,
}
impl Role {
    const ALL: [Self; 5] = [
        Self::Viewer,
        Self::Chat,
        Self::Scenes,
        Self::Operator,
        Self::Administrator,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Viewer => "Viewer",
            Self::Chat => "Chat",
            Self::Scenes => "Scene operator",
            Self::Operator => "Operator",
            Self::Administrator => "Administrator",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Viewer => "Watch the studio, chat and interactive activity.",
            Self::Chat => "Read chat and send replies.",
            Self::Scenes => "Use scene controls and watch the studio.",
            Self::Operator => "Manage scenes, live output and extensions.",
            Self::Administrator => "Full control, including device access.",
        }
    }
    fn grants(self) -> BTreeSet<P> {
        match self {
            Self::Viewer => [P::StudioView, P::InteractiveView, P::ChatRead]
                .into_iter()
                .collect(),
            Self::Chat => [P::ChatRead, P::ChatSend].into_iter().collect(),
            Self::Scenes => [P::StudioView, P::ScenesManage].into_iter().collect(),
            Self::Operator => P::ALL
                .into_iter()
                .filter(|p| !matches!(p, P::AccessManage | P::ExtensionsInstall))
                .collect(),
            Self::Administrator => P::ALL.into_iter().collect(),
        }
    }
}
fn role_name(grants: &BTreeSet<P>) -> &'static str {
    Role::ALL
        .into_iter()
        .find(|role| role.grants() == *grants)
        .map(Role::title)
        .unwrap_or("Custom access")
}
#[derive(Clone)]
struct ReadyInvitation {
    code: String,
    role: &'static str,
    permission_count: usize,
}

#[component]
fn RolePicker(mut grants: Signal<BTreeSet<P>>, available: BTreeSet<P>, busy: bool) -> Element {
    rsx! {div {class:"access-role-list",role:"group","aria-label":"Invitation role",
        for role in Role::ALL.into_iter().filter(|r|r.grants().is_subset(&available)) {
            ActionButton {class:if *grants.read()==role.grants(){"access-role selected"}else{"access-role"},pressed:Some(*grants.read()==role.grants()),disabled:busy,onclick:move |_|grants.set(role.grants()),
                span {class:"access-radio","aria-hidden":"true"}
                span {class:"access-role-copy",strong {"{role.title()}"}span {"{role.description()}"}}
            }
        }
    }}
}
#[component]
fn PermissionEditor(
    mut grants: Signal<BTreeSet<P>>,
    available: BTreeSet<P>,
    busy: bool,
) -> Element {
    rsx! {div {class:"access-permissions",for (label,permissions) in [
        ("Viewing",vec![P::StudioView,P::InteractiveView,P::ChatRead]),
        ("Production",vec![P::ChatSend,P::ScenesManage,P::MediaManage,P::WorkspaceManage,P::InteractiveManage]),
        ("Extensions",vec![P::ExtensionsConfigure,P::ExtensionsRun,P::ExtensionsInstall]),
        ("Administration",vec![P::AccessManage]),
    ] {if permissions.iter().any(|p|available.contains(p)) {div {class:"access-permission-group",h4 {"{label}"}
        for permission in permissions.into_iter().filter(|p|available.contains(p)) {
            ActionButton {class:if grants.read().contains(&permission){"access-permission selected"}else{"access-permission"},pressed:Some(grants.read().contains(&permission)),disabled:busy,onclick:move |_|{let mut values=grants.write();if !values.remove(&permission){values.insert(permission);}},
                span {class:"access-check","aria-hidden":"true",if grants.read().contains(&permission){"✓"}}
                span {"{permission.label()}"}
            }
        }
    }}}}}
}

#[component]
pub(crate) fn Panel(project: Signal<Workspace>) -> Element {
    let mut state = use_context::<AccessState>();
    let clipboard = try_consume_context::<crate::live_client::LiveClient>();
    let mut name = use_signal(|| {
        if cfg!(target_arch = "wasm32") {
            "Browser".to_owned()
        } else {
            "Desktop".to_owned()
        }
    });
    let mut code = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(String::new);
    let mut invitation = use_signal(|| None::<ReadyInvitation>);
    let mut copied = use_signal(|| false);
    let mut show_code = use_signal(|| false);
    let mut advanced = use_signal(|| false);
    let mut devices = use_signal(Vec::<Device>::new);
    let mut grants = use_signal(|| Role::Viewer.grants());
    let endpoint = use_memo(move || crate::media::server_url(&project.read(), "/api/access"));
    let identity = use_memo(move || {
        state.session.read().as_ref().map(|s| {
            (
                s.server_id.clone(),
                s.device.id.clone(),
                s.device.permissions.clone(),
                s.user.id.clone(),
            )
        })
    });
    use_effect(move || {
        let identity = identity();
        let available = identity.as_ref().map(|s| s.2.clone()).unwrap_or_default();
        grants.set(
            Role::Viewer
                .grants()
                .intersection(&available)
                .copied()
                .collect(),
        );
        invitation.set(None);
        code.set(String::new());
        error.set(String::new());
        busy.set(false);
        advanced.set(false);
        copied.set(false);
        show_code.set(false);
    });
    use_resource(move || {
        let target = endpoint();
        let identity = identity();
        async move {
            devices.set(vec![]);
            if !identity
                .as_ref()
                .is_some_and(|s| s.2.contains(&P::AccessManage))
            {
                return;
            }
            let Ok(target) = target else {
                return;
            };
            loop {
                if let Ok(result) = get::<Devices>(&format!("{target}/devices")).await {
                    devices.set(result.devices);
                }
                crate::scene_preview::pause(5000).await;
            }
        }
    });
    let session = state.session.read().clone();
    let available = session
        .as_ref()
        .map(|s| s.device.permissions.clone())
        .unwrap_or_default();
    let administrator = available.contains(&P::AccessManage);
    let current_id = session
        .as_ref()
        .map(|s| s.device.id.clone())
        .unwrap_or_default();
    let warning = if !error.read().is_empty() {
        error.read().clone()
    } else {
        state.error.read().clone()
    };
    let mut connect = move |_: ()| {
        if *busy.peek() {
            return;
        }
        let target = endpoint();
        let secret = code.peek().trim().to_owned();
        let device_name = name.peek().clone();
        busy.set(true);
        error.set(String::new());
        code.set(String::new());
        spawn(async move {
            let result = match target.clone() {
                Ok(target) => pair_device(&format!("{target}/pair"), secret, device_name).await,
                Err(e) => Err(e),
            };
            if endpoint.peek().as_ref() != target.as_ref() {
                return;
            }
            match result {
                Ok(session) => state.session.set(Some(session)),
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let disconnect = move |_: ()| {
        let target = endpoint();
        let actor = identity();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let target = target.clone()?;
                let response = request(reqwest::Method::POST, &format!("{target}/logout"))?
                    .json(&serde_json::json!({}))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                    .map_err(|_| "Cannot disconnect this device.")?;
                if !response.status().is_success() {
                    return Err("Cannot disconnect this device.".to_owned());
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let target = target.clone();
                    crate::native_credentials::run(move || {
                        acheflow_client::access::forget(&target)
                    })
                    .await?;
                }
                #[cfg(target_arch = "wasm32")]
                TOKENS.with(|tokens| {
                    tokens
                        .borrow_mut()
                        .remove(&base(&target).unwrap_or_default())
                });
                Ok::<_, String>(())
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *identity.peek() != actor {
                return;
            }
            match result {
                Ok(()) => state.session.set(None),
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let create = move |_: ()| {
        if *busy.peek() {
            return;
        }
        let target = endpoint();
        let actor = identity();
        let selected = grants.peek().clone();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let url = format!("{}/invitations", target.clone()?);
                decode::<acheflow_media_api::access::Invitation>(
                    request(reqwest::Method::POST, &url)?
                        .json(&InviteRequest {
                            permissions: selected.clone(),
                        })
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await
                        .map_err(|_| "Cannot create an invitation.")?,
                )
                .await
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *identity.peek() != actor {
                return;
            }
            match result {
                Ok(result) => {
                    invitation.set(Some(ReadyInvitation {
                        code: result.code,
                        role: role_name(&selected),
                        permission_count: selected.len(),
                    }));
                    copied.set(false);
                    show_code.set(false);
                }
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let mut revoke = move |id: String| {
        let target = endpoint();
        let actor = identity();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let url = format!("{}/devices/{id}", target.clone()?);
                let response = request(reqwest::Method::DELETE, &url)?
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                    .map_err(|_| "Cannot revoke device access.")?;
                if !response.status().is_success() {
                    return Err("Cannot revoke device access.".to_owned());
                }
                Ok::<_, String>(())
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *identity.peek() != actor {
                return;
            }
            match result {
                Ok(()) => {
                    devices.write().retain(|d| d.id != id);
                    if state
                        .session
                        .peek()
                        .as_ref()
                        .is_some_and(|s| s.device.id == id)
                    {
                        state.session.set(None);
                    }
                }
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    rsx! {div {class:"access-page",
        div {class:"access-intro",div {h2 {"Your studio, together"}p {"Connect your devices and choose who can watch or take control."}}
            span {class:if session.is_some(){"access-status connected"}else{"access-status"},span {class:"access-status-dot"}if session.is_some(){"Connected"}else{"Not connected"}}
        }
        if let Some(session)=session.as_ref() {
            div {class:"access-current",span {class:"access-device-icon","aria-hidden":"true","◈"}div {strong {"{session.device.name}"}p {if session.device.id=="owner"{"This computer owns the server."}else{"This device is connected to the server."}}}
                span {class:"access-role-badge","{role_name(&session.device.permissions)}"}
                if session.device.id!="owner" {ActionButton {class:"access-text-button",disabled:busy(),onclick:disconnect,"Disconnect"}}
            }
        } else {
            section {class:"access-card access-connect-card",div {class:"access-card-heading",span {class:"access-connect-icon","aria-hidden":"true","↔"}h3 {"Connect this device"}p {"Enter a device link from your profile, or an invitation from an administrator."}}
                div {class:"access-connect-form","data-text-submit":"",onsubmit:move |event|{event.prevent_default();if !busy() && !code.peek().is_empty(){connect(());}},
                    label {class:"field",span {"Device name"}input {r#type:"text",maxlength:80,value:name(),disabled:busy(),oninput:move|e|name.set(e.value())}}
                    label {class:"field",span {"Invitation or device-link code"}input {r#type:"password",autocomplete:"off",maxlength:64,placeholder:"Paste your invitation code",value:code(),disabled:busy(),oninput:move|e|code.set(e.value()),onkeydown:move|event|{if event.key()==Key::Named(NamedKey::Enter) && !code.peek().is_empty(){event.prevent_default();connect(());}}}}
                    ActionButton {id:"access-connect",class:"primary",disabled:busy() || code.read().is_empty(),onclick:connect,if busy(){"Connecting…"}else{"Connect device"}}
                }
                p {class:"access-footnote","Invitations are private, work once and expire after 10 minutes."}
            }
        }
        if session.is_some() {identity::Identity {project}}
        if !warning.is_empty(){div {class:"access-error",role:"alert","{warning}"}}
        if administrator {div {class:"access-layout",
            section {class:"access-card",div {class:"access-card-heading",span {class:"access-eyebrow","SHARE YOUR STUDIO"}h3 {"Invite a participant"}p {"Create a separate profile for someone else. Choose their access below."}}
                if let Some(ready)=invitation.read().clone() {
                    div {class:"access-invitation-ready",span {class:"access-ready-icon","aria-hidden":"true","✓"}h4 {"Your invitation is ready"}
                        p {"{ready.role} · {ready.permission_count} permissions"}
                        div {class:"access-invitation-code",input {r#type:if show_code(){"text"}else{"password"},readonly:true,value:ready.code.clone(),"aria-label":"One-time invitation code"}
                            ActionButton {class:"access-text-button",onclick:move |_|show_code.set(!show_code()),if show_code(){"Hide"}else{"Show"}}
                        }
                        ActionButton {class:"primary access-copy",onclick:move |_|{let value=invitation.peek().as_ref().map(|v|v.code.clone());if let Some(value)=value {spawn(async move {if let Some(client)=clipboard {if client.copy_text(value).await {copied.set(true);}}else{error.set("Select the code to copy it.".into());}});}},if copied(){"Copied ✓"}else{"Copy invitation"}}
                        p {class:"access-footnote","One use. Expires 10 minutes after creation."}
                        ActionButton {class:"access-text-button",onclick:move |_|invitation.set(None),"Create another invitation"}
                    }
                } else {
                    RolePicker {grants,available:available.clone(),busy:busy()}
                    div {class:"access-custom",ActionButton {class:"access-custom-toggle",expanded:Some(advanced()),disabled:busy(),onclick:move |_|advanced.set(!advanced()),span {"Customize permissions"}span {class:"access-custom-count","{grants.read().len()} selected"}span {"aria-hidden":"true",if advanced(){"−"}else{"+"}}}
                        if advanced(){PermissionEditor {grants,available:available.clone(),busy:busy()}}
                    }
                    div {class:"access-invite-footer",ActionButton {id:"access-create-invitation",class:"primary",disabled:busy() || grants.read().is_empty(),onclick:create,if busy(){"Creating…"}else{"Create invitation"}}p {"Private · single use · 10 minutes"}}
                }
            }
            section {class:"access-card access-devices-card",div {class:"access-card-heading access-devices-heading",div {span {class:"access-eyebrow","CONNECTED TO THIS SERVER"}h3 {"Devices"}}span {class:"access-device-count","{devices.read().len()}"}}
                if devices.read().is_empty(){div {class:"access-empty",span {class:"access-empty-icon","aria-hidden":"true","◈"}h4 {"Ready for another device"}p {"Create an invitation to connect a phone or another computer."}}}
                for device in devices.read().clone(){div {class:"access-device-row",span {class:"access-device-avatar","aria-hidden":"true","◈"}div {class:"access-device-copy",strong {"{device.name}"}p {"{role_name(&device.permissions)}"}if device.id==current_id {span {class:"access-this-device","This device"}}}
                    ActionButton {class:"access-revoke",disabled:busy() || !device.permissions.is_subset(&available),onclick:move |_|revoke(device.id.clone()),"Remove"}
                }}
                p {class:"access-devices-note","Removing a device ends its access to this server."}
            }
        }}
    }}
}
