//! Shared connection settings and native-only process controls.
use crate::project::Workspace;
use acheflow_media_api::server::ServerInfo;
use dioxus_native::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ServerPreferences {
    pub start_with_client: bool,
    pub keep_running_on_exit: bool,
    pub http_port: u16,
    pub rtmp_port: u16,
}
impl Default for ServerPreferences {
    fn default() -> Self {
        Self {
            start_with_client: false,
            keep_running_on_exit: false,
            http_port: 21295,
            rtmp_port: 1935,
        }
    }
}
impl ServerPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if [self.http_port, self.rtmp_port].contains(&0) || self.http_port == self.rtmp_port {
            return Err("Enter two different ports from 1 to 65535".into());
        }
        Ok(())
    }
}

/// Blitz does not synthesize button/checkbox activation for Space and Enter.
/// Prevent the default so the same handler also fires once in browsers.
#[component]
fn ActionButton(
    #[props(default)] id: &'static str,
    #[props(default)] disabled: bool,
    expanded: Option<bool>,
    onclick: EventHandler<()>,
    children: Element,
) -> Element {
    rsx! {button {id,disabled,"aria-expanded":expanded.map(|v|v.to_string()),
        onclick:move |_|{if !disabled {onclick.call(());}},
        onkeydown:move |event|{
            if !disabled && !event.is_auto_repeating() && (event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into())) {
                event.prevent_default();onclick.call(());
            }
        },
        {children}
    }}
}
#[derive(Clone, Copy, PartialEq)]
struct ServerState {
    available: Signal<Option<bool>>,
    info: Signal<Option<ServerInfo>>,
    error: Signal<String>,
    #[cfg(not(target_arch = "wasm32"))]
    busy: Signal<bool>,
    #[cfg(not(target_arch = "wasm32"))]
    operation: Signal<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    process: Signal<acheflow_client::server::Status>,
    #[cfg(not(target_arch = "wasm32"))]
    external: Signal<Option<ServerInfo>>,
    #[cfg(not(target_arch = "wasm32"))]
    logs: Signal<Vec<String>>,
}
pub(crate) fn provide(project: Signal<Workspace>, catalog: Signal<crate::workspaces::Catalog>) {
    let mut state = ServerState {
        available: use_signal(|| None),
        info: use_signal(|| None),
        error: use_signal(String::new),
        #[cfg(not(target_arch = "wasm32"))]
        busy: use_signal(|| false),
        #[cfg(not(target_arch = "wasm32"))]
        operation: use_signal(|| 0),
        #[cfg(not(target_arch = "wasm32"))]
        process: use_signal(Default::default),
        #[cfg(not(target_arch = "wasm32"))]
        external: use_signal(|| None),
        #[cfg(not(target_arch = "wasm32"))]
        logs: use_signal(Vec::new),
    };
    use_context_provider(|| state);
    let target = use_memo(move || crate::media::server_url(&project.read(), "/api/server/info"));
    use_resource(move || {
        let target = target();
        state.available.set(None);
        state.info.set(None);
        async move {
            loop {
                let result = match &target {
                    Ok(url) => fetch_info(url).await,
                    Err(_) => None,
                };
                state.available.set(Some(result.is_some()));
                state.info.set(result);
                pause().await;
            }
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    native::provide(state, project, catalog);
    #[cfg(target_arch = "wasm32")]
    let _ = catalog;
}
async fn fetch_info(url: &str) -> Option<ServerInfo> {
    reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}
async fn pause() {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(2000).await;
}
#[cfg(not(target_arch = "wasm32"))]
fn local_target(workspace: &Workspace, port: u16) -> bool {
    let Ok(url) = crate::media::server_url(workspace, "")
        .and_then(|url| url::Url::parse(&url).map_err(|e| e.to_string()))
    else {
        return false;
    };
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain("localhost")) => true,
        _ => false,
    };
    local
        && url.scheme() == "http"
        && (workspace.server_url.is_empty() || url.port_or_known_default() == Some(port))
        && url.path() == "/"
}

#[component]
pub(crate) fn ServerSettings(
    project: Signal<Workspace>,
    catalog: Signal<crate::workspaces::Catalog>,
    storage_status: Signal<String>,
) -> Element {
    let mut state = use_context::<ServerState>();
    let mut address = use_signal(|| project.peek().server_url.clone());
    let target =
        use_memo(move || crate::media::server_url(&project.read(), "").unwrap_or_default());
    use_effect(move || address.set(target()));
    let availability = match (state.available)() {
        None => "Checking API…",
        Some(true) => "API available",
        Some(false) => "API unavailable",
    };
    rsx! {
        section {class:"panel server-settings",
            div {class:"panel-heading",div {h2 {"Server"}p {"Choose the server for this workspace."}}span {class:"badge",role:"status","{availability}"}}
            div {class:"padded",
                div {class:"server-connection",
                    label {class:"field",span {"Workspace server address"}
                        input {r#type:"text","aria-label":"Workspace server address",value:address(),disabled:!crate::media::can_choose_server(),oninput:move|e|address.set(e.value())}
                    }
                    ActionButton {disabled:!crate::media::can_choose_server(),onclick:move |_|{
                        match acheflow_media_api::workspace::normalize_server_url(&address.peek()) {
                            Ok(url)=>{let mut project=project;project.write().server_url=url;state.error.set(String::new());},
                            Err(error)=>state.error.set(error),
                        }
                    },"Connect"}
                }
                if let Some(info)=(state.info)() {p {class:"form-help","Server v{info.version}"}}
                if !crate::media::can_choose_server() {p {class:"form-help","This browser client is connected to the server hosting it."}}
                {local_controls(project, catalog, state)}
                if !(state.error)().is_empty() {p {class:"error",role:"alert","{state.error}"}}
                p {class:"form-help",role:"status","{storage_status}"}
            }
        }
    }
}
fn local_controls(
    project: Signal<Workspace>,
    catalog: Signal<crate::workspaces::Catalog>,
    state: ServerState,
) -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rsx! {LocalControls {project,catalog,state}}
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (project, catalog, state);
        rsx! {p {class:"form-help","To start a local server, open the desktop app."}}
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
fn LocalControls(
    mut project: Signal<Workspace>,
    mut catalog: Signal<crate::workspaces::Catalog>,
    state: ServerState,
) -> Element {
    use acheflow_client::server::{LocalServer, Phase};
    use std::sync::Arc;
    let manager = use_context::<Option<Arc<LocalServer>>>();
    let preferences = catalog.read().server_preferences();
    let process = (state.process)();
    let owned = process.pid.is_some();
    let external = (state.external)().filter(|info| Some(info.process_id) != process.pid);
    let busy = (state.busy)();
    let mut http = use_signal(|| preferences.http_port.to_string());
    let mut rtmp = use_signal(|| preferences.rtmp_port.to_string());
    let mut ports_error = use_signal(String::new);
    let mut show_logs = use_signal(|| false);
    let label = if external.is_some() && !owned {
        "Running externally"
    } else {
        match process.phase {
            Phase::Stopped => "Stopped",
            Phase::Starting => "Starting…",
            Phase::Ready => "Ready",
            Phase::Unavailable => "Process running · API unavailable",
            Phase::Stopping => "Stopping…",
            Phase::Exited => "Exited",
            Phase::Failed => "Failed",
            Phase::Detached => "Running externally",
        }
    };
    let keyboard_manager = manager.clone();
    let start_manager = manager.clone();
    let stop_manager = manager.clone();
    let restart_manager = manager.clone();
    let preferences_start = preferences.clone();
    let preferences_stop = preferences.clone();
    let preferences_restart = preferences.clone();
    let can_connect = process.phase == Phase::Ready || external.is_some();
    let local_endpoint = format!("http://127.0.0.1:{}", preferences.http_port);
    let version = process
        .info
        .as_ref()
        .or(external.as_ref())
        .map(|info| info.version.clone())
        .unwrap_or_else(|| "—".into());
    rsx! {
        div {class:"local-server",
            div {class:"server-section-heading",h3 {"Local server"}span {class:"badge",role:"status","{label}"}}
            div {class:"server-actions",
                ActionButton {id:"local-server-start",disabled:busy||owned||external.is_some()||manager.is_none(),onclick:move|_|native::action(state,project,preferences_start.clone(),"start",start_manager.clone()),"Start server"}
                ActionButton {id:"local-server-stop",disabled:!owned||process.phase==Phase::Stopping,onclick:move|_|native::action(state,project,preferences_stop.clone(),"stop",stop_manager.clone()),"Stop server"}
                ActionButton {id:"local-server-restart",disabled:busy||!owned,onclick:move|_|native::action(state,project,preferences_restart.clone(),"restart",restart_manager.clone()),"Restart server"}
                ActionButton {id:"use-local-server",disabled:busy||!can_connect,onclick:move|_|project.write().server_url=local_endpoint.clone(),"Use local server"}
            }
            if external.is_some()&&!owned {p {class:"form-help","This server was started outside this client. It can be used here; stop it from its owner."}}
            if owned {p {class:"form-help","Owned process: {process.pid.unwrap_or_default()}"}}
            p {class:"form-help","Endpoint: http://127.0.0.1:{preferences.http_port} · Version: {version}"}
            div {class:"server-ports",
                label {class:"field",span {"HTTP port"}input {r#type:"text",inputmode:"numeric",pattern:"[0-9]*","aria-label":"Local HTTP port",value:http(),disabled:owned||busy,oninput:move|e|http.set(e.value())}}
                label {class:"field",span {"RTMP port"}input {r#type:"text",inputmode:"numeric",pattern:"[0-9]*","aria-label":"Local RTMP port",value:rtmp(),disabled:owned||busy,oninput:move|e|rtmp.set(e.value())}}
                ActionButton {disabled:owned||busy,onclick:move|_|{
                    let mut prefs=catalog.peek().server_preferences();
                    let parsed=http.peek().parse::<u16>().ok().zip(rtmp.peek().parse::<u16>().ok());
                    if let Some((h,r))=parsed {prefs.http_port=h;prefs.rtmp_port=r;}
                    match parsed.ok_or_else(||"Enter two different ports from 1 to 65535".to_string()).and_then(|_|prefs.validate()) {
                        Ok(())=>{catalog.write().set_server_preferences(prefs);ports_error.set(String::new());},
                        Err(error)=>ports_error.set(error),
                    }
                },"Save ports"}
            }
            if !ports_error().is_empty() {p {class:"error",role:"alert","{ports_error}"}}
            div {class:"server-preferences",
                label {input {r#type:"checkbox",id:"server-auto-start",checked:preferences.start_with_client,onkeydown:move|e|{if !e.is_auto_repeating() && (e.key()==Key::Character(" ".into())||e.key()==Key::Named(NamedKey::Enter)) {e.prevent_default();let mut prefs=catalog.peek().server_preferences();prefs.start_with_client= !prefs.start_with_client;catalog.write().set_server_preferences(prefs);}},oninput:move|e|{let mut prefs=catalog.peek().server_preferences();prefs.start_with_client=e.checked();catalog.write().set_server_preferences(prefs);}}span {"Start local server with the desktop app"}}
                label {input {r#type:"checkbox",id:"server-keep-running",checked:preferences.keep_running_on_exit,onkeydown:move|e|{if !e.is_auto_repeating() && (e.key()==Key::Character(" ".into())||e.key()==Key::Named(NamedKey::Enter)) {e.prevent_default();let mut prefs=catalog.peek().server_preferences();prefs.keep_running_on_exit= !prefs.keep_running_on_exit;if let Some(manager)=&keyboard_manager {manager.set_keep_running_on_exit(prefs.keep_running_on_exit);}catalog.write().set_server_preferences(prefs);}},oninput:move|e|{let mut prefs=catalog.peek().server_preferences();prefs.keep_running_on_exit=e.checked();if let Some(manager)=&manager {manager.set_keep_running_on_exit(prefs.keep_running_on_exit);}catalog.write().set_server_preferences(prefs);}}span {"Keep local server running when the app closes"}}
            }
            if let Some(error)=process.error && (state.error)().is_empty() {p {class:"error",role:"alert","{error}"}}
            if process.forced_stop {p {class:"form-help","The last process needed forced termination."}}
            ActionButton {expanded:Some(show_logs()),onclick:move|_|show_logs.set(!show_logs()),if show_logs(){"Hide recent logs"}else{"Show recent logs"}}
            if show_logs() {pre {class:"server-logs",tabindex:"0","aria-label":"Recent local server logs",if (state.logs)().is_empty(){"No output from this client’s server yet."}else{"{(state.logs)().join(\"\")}"}}}
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
