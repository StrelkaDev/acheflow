//! Host-rendered local guest panels. All account logic is executed by the extension.
use acheflow_client::{catalog::ClientCatalog, local::LocalSession};
use acheflow_extension_api::{ExtensionPanel, PanelWidget};
use dioxus_core::Task;
use dioxus_native::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};
type Sessions = BTreeMap<(String, String), LocalSession>;
pub(crate) fn provide_accounts(project: Signal<crate::project::Workspace>) {
    let mut sessions = use_signal(Sessions::new);
    use_context_provider(|| sessions);
    use_effect(move || {
        let project = project.read();
        sessions.write().retain(|(owner, id), _| {
            owner == &project.id && project.client_extensions.get(id).is_some_and(|s| s.enabled)
        });
    });
    let server = use_memo(move || project.read().server_url.clone());
    use_effect(move || {
        let _ = server();
        sessions.write().clear();
    });
}
fn now() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
fn environment() -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    {
        json!({"browser":false,"callback_uri":acheflow_client::browser::CALLBACK})
    }
    #[cfg(target_arch = "wasm32")]
    {
        let callback = web_sys::window()
            .and_then(|w| w.location().href().ok())
            .and_then(|href| url::Url::parse(&href).ok())
            .and_then(|url| url.join("oauth-callback.html").ok())
            .map(|url| url.to_string())
            .unwrap_or_default();
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_default();
        json!({"browser":true,"callback_uri":callback,"origin":origin})
    }
}
#[component]
pub(crate) fn ClientAuthorization(id: String, name: String) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let sessions = use_context::<Signal<Sessions>>();
    let panel = use_signal(|| None::<ExtensionPanel>);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut drafts = use_signal(BTreeMap::<String, String>::new);
    let mut task = use_signal(|| None::<Task>);
    let mut restart = use_signal(|| 0u64);
    use_drop(move || {
        if let Some(task) = task.peek().as_ref() {
            task.cancel();
        }
    });
    let identity = use_memo({
        let id = id.clone();
        move || {
            let p = client.project.read();
            (
                p.id.clone(),
                p.server_url.clone(),
                p.client_extensions
                    .get(&id)
                    .map(|s| s.config.clone())
                    .unwrap_or_default(),
            )
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    let native = use_signal(|| native::Mailbox::new(error));
    use_effect(move || {
        let _ = identity();
        if let Some(running) = task.take() {
            running.cancel();
        }
        drafts.write().clear();
    });
    let signals = PanelSignals {
        client,
        sessions,
        panel,
        error,
        busy,
        drafts,
        task,
        restart,
        identity,
        #[cfg(not(target_arch = "wasm32"))]
        native,
    };
    #[cfg(not(target_arch = "wasm32"))]
    native::register(id.clone(), signals);
    let installed = &catalog.installed[&id];
    #[cfg(target_arch = "wasm32")]
    let mut adapter = use_signal(|| None::<wasm_bindgen::JsValue>);
    #[cfg(target_arch = "wasm32")]
    let _prepare = use_resource({
        let source = installed.browser_auth.clone();
        move || {
            let source = source.clone();
            async move {
                if let Some(source) = source {
                    match browser::prepare(&source).await {
                        Ok(module) => adapter.set(Some(module)),
                        Err(_) => error.set(
                            "Platform sign-in could not load. Reopen this card or enter a token."
                                .into(),
                        ),
                    }
                }
            }
        }
    });
    use_initialization(id.clone(), installed, signals);
    let action = Callback::new({
        let id = id.clone();
        move |action_id: String| {
            dispatch(
                &id,
                action_id,
                signals,
                #[cfg(target_arch = "wasm32")]
                adapter.peek().clone(),
            )
        }
    });
    use_ticks(id.clone(), signals);
    rsx! {div{class:"client-authorization",
        if let Some(panel)=panel.read().as_ref(){
            h3{"{panel.title}"}
            for widget in panel.widgets.clone(){{render_widget(widget,id.clone(),signals,action)}}
        }
        p{class:"form-help extension-storage-note",if cfg!(target_arch="wasm32"){"Tokens stay in this browser session and are never sent to the server."}else{"Tokens are kept in the system credential store. Disconnect removes the saved account."}}
        if busy(){p{role:"status","Waiting for {name}…"}if task.read().is_some(){button{onclick:move|_|{if let Some(task)=task.take(){task.cancel();}busy.set(false);restart+=1;error.set("Operation cancelled.".into());},"Cancel"}}}
        if !error().is_empty(){p{class:"field-error",role:"alert","{error}"}}
    }}
}

type Owner = (String, String, BTreeMap<String, String>);
#[derive(Clone, Copy)]
struct PanelSignals {
    client: crate::control_client::ControlClient,
    sessions: Signal<Sessions>,
    panel: Signal<Option<ExtensionPanel>>,
    error: Signal<String>,
    busy: Signal<bool>,
    drafts: Signal<BTreeMap<String, String>>,
    task: Signal<Option<Task>>,
    restart: Signal<u64>,
    identity: Memo<Owner>,
    #[cfg(not(target_arch = "wasm32"))]
    native: Signal<native::Mailbox>,
}
fn use_initialization(
    id: String,
    installed: &acheflow_client::catalog::Installed,
    signals: PanelSignals,
) {
    let PanelSignals {
        mut sessions,
        mut panel,
        mut error,
        mut busy,
        restart,
        identity,
        ..
    } = signals;
    let _initialize = use_resource({
        let id = id.clone();
        let manifest = installed.manifest.clone();
        let module = installed.module.clone();
        move || {
            let _version = restart();
            let owner = identity();
            let id = id.clone();
            let manifest = manifest.clone();
            let module = module.clone();
            let existing = sessions.write().remove(&(owner.0.clone(), id.clone()));
            #[cfg(not(target_arch = "wasm32"))]
            signals.native.peek().begin_initialization();
            busy.set(true);
            async move {
                let result = async {
                    let mut session = existing
                        .map(Ok)
                        .unwrap_or_else(|| LocalSession::new(manifest, &module, owner.2.clone()))?;
                    session.config = owner.2.clone();
                    session.bind_workspace(&owner.0)?;
                    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
                    session.use_system_storage(&owner.1, &owner.0)?;
                    let effects =
                        session.step(json!({"type":"local_start"}), &environment(), now())?;
                    drive(
                        session,
                        effects,
                        #[cfg(target_arch = "wasm32")]
                        None,
                        #[cfg(not(target_arch = "wasm32"))]
                        signals.native.peek().clone(),
                        #[cfg(not(target_arch = "wasm32"))]
                        native::Context::new(signals, owner.clone(), _version),
                    )
                    .await
                }
                .await;
                if identity() == owner && restart() == _version {
                    #[cfg(not(target_arch = "wasm32"))]
                    signals.native.peek().finish_initialization();
                    match result {
                        Ok((mut session, next)) => {
                            if let Err(reason) = session.flush_storage() {
                                error.set(reason);
                            } else {
                                error.set(String::new());
                            }
                            sessions.write().insert((owner.0, id), session);
                            panel.set(next);
                        }
                        Err(reason) => error.set(reason),
                    }
                    busy.set(false);
                }
            }
        }
    });
}
fn dispatch(
    id: &str,
    action_id: String,
    signals: PanelSignals,
    #[cfg(target_arch = "wasm32")] adapter: Option<wasm_bindgen::JsValue>,
) {
    let PanelSignals {
        mut sessions,
        mut panel,
        mut error,
        busy,
        mut drafts,
        identity,
        ..
    } = signals;
    let provider = id.to_owned();

    if busy() {
        return;
    }
    let owner = identity();
    let key = (owner.0.clone(), provider.clone());
    let Some(mut session) = sessions.write().remove(&key) else {
        return;
    };
    for (key, value) in drafts.take() {
        if let Err(reason) = session.set_secret(&key, value) {
            error.set(reason);
            sessions
                .write()
                .insert((owner.0, provider.clone()), session);
            return;
        }
    }
    let effects = match session.step(
        json!({"type":"local_action","action_id":action_id}),
        &environment(),
        now(),
    ) {
        Ok(effects) => effects,
        Err(reason) => {
            error.set(reason);
            return;
        }
    };
    if !effects.iter().any(|effect| {
        matches!(
            effect["type"].as_str(),
            Some("http" | "browser" | "browser_module")
        )
    }) {
        for effect in effects {
            if effect["type"] == "panel" {
                match serde_json::from_value(effect["panel"].clone()) {
                    Ok(next) => panel.set(Some(next)),
                    Err(_) => error.set("Invalid local panel.".into()),
                }
            }
        }
        if let Err(reason) = session.flush_storage() {
            error.set(reason);
        }
        sessions.write().insert(key, session);
        return;
    }
    #[cfg(target_arch = "wasm32")]
    let started = match browser::start_effect(&session, &effects, adapter, &environment()) {
        Ok(started) => started,
        Err(reason) => {
            error.set(reason);
            sessions.write().insert(key, session);
            return;
        }
    };
    start_operation(
        provider,
        session,
        effects,
        signals,
        #[cfg(target_arch = "wasm32")]
        started,
    );
}
fn start_operation(
    provider: String,
    session: LocalSession,
    effects: Vec<Value>,
    signals: PanelSignals,
    #[cfg(target_arch = "wasm32")] started: Option<browser::Login>,
) {
    let PanelSignals {
        client,
        mut sessions,
        mut panel,
        mut error,
        mut busy,
        mut task,
        identity,
        ..
    } = signals;
    let owner = identity();
    busy.set(true);
    error.set(String::new());
    #[cfg(not(target_arch = "wasm32"))]
    let native_mailbox = signals.native.peek().clone();
    #[cfg(not(target_arch = "wasm32"))]
    let native_context = native::Context::new(signals, owner.clone(), (signals.restart)());
    task.set(Some(spawn(async move {
        let result = drive(
            session,
            effects,
            #[cfg(target_arch = "wasm32")]
            started,
            #[cfg(not(target_arch = "wasm32"))]
            native_mailbox,
            #[cfg(not(target_arch = "wasm32"))]
            native_context,
        )
        .await;
        if identity() == owner
            && client
                .project
                .peek()
                .client_extensions
                .get(&provider)
                .is_some_and(|s| s.enabled)
        {
            match result {
                Ok((mut session, next)) => {
                    if let Err(reason) = session.flush_storage() {
                        error.set(reason);
                    }
                    sessions.write().insert((owner.0, provider), session);
                    panel.set(next);
                }
                Err(reason) => error.set(reason),
            }
        }
        busy.set(false);
        task.set(None);
    })));
}
fn use_ticks(id: String, signals: PanelSignals) {
    let PanelSignals {
        mut sessions,
        mut panel,
        mut error,
        mut busy,
        identity,
        ..
    } = signals;
    let _ticks = use_resource({
        let id = id.clone();
        move || {
            let id = id.clone();
            async move {
                loop {
                    crate::scene_preview::pause(30_000).await;
                    if *busy.peek() {
                        continue;
                    }
                    let owner = identity.peek().clone();
                    let Some(mut session) = sessions.write().remove(&(owner.0.clone(), id.clone()))
                    else {
                        continue;
                    };
                    let effects =
                        match session.step(json!({"type":"local_tick"}), &environment(), now()) {
                            Ok(effects) => effects,
                            Err(reason) => {
                                error.set(reason);
                                sessions.write().insert((owner.0, id.clone()), session);
                                continue;
                            }
                        };
                    busy.set(true);
                    let result = drive(
                        session,
                        effects,
                        #[cfg(target_arch = "wasm32")]
                        None,
                        #[cfg(not(target_arch = "wasm32"))]
                        signals.native.peek().clone(),
                        #[cfg(not(target_arch = "wasm32"))]
                        native::Context::new(signals, owner.clone(), (signals.restart)()),
                    )
                    .await;
                    if identity.peek().clone() == owner {
                        match result {
                            Ok((mut session, next)) => {
                                if let Err(reason) = session.flush_storage() {
                                    error.set(reason);
                                }
                                sessions.write().insert((owner.0, id.clone()), session);
                                if panel.peek().as_ref() != next.as_ref() {
                                    panel.set(next);
                                }
                            }
                            Err(reason) => error.set(reason),
                        }
                    }
                    busy.set(false);
                }
            }
        }
    });
}
#[component]
fn LocalSetting(extension: String, id: String) -> Element {
    let catalog = use_context::<Rc<ClientCatalog>>();
    let setting = catalog
        .installed
        .get(&extension)
        .and_then(|installed| {
            installed
                .manifest
                .settings
                .iter()
                .find(|s| s.id == id && !s.secret && !s.toggle)
        })
        .cloned();
    match setting {
        Some(setting) => rsx! {crate::interaction::ExtensionSetting{extension,setting,local:true}},
        None => rsx! {p{class:"field-error",role:"alert","Unknown public extension setting."}},
    }
}

fn render_widget(
    widget: PanelWidget,
    extension: String,
    signals: PanelSignals,
    action: Callback<String>,
) -> Element {
    let PanelSignals {
        busy, mut drafts, ..
    } = signals;
    match widget {
        PanelWidget::Label { text } => rsx! {p{class:"form-help","{text}"}},
        PanelWidget::Button { id, label, primary } => {
            let key_id = id.clone();
            rsx! {button{class:if primary{"local-extension-action primary"}else{"local-extension-action"},disabled:busy(),onclick:move|_|action.call(id.clone()),onkeydown:move|event|{if event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into()){event.prevent_default();action.call(key_id.clone());}},"{label}"}}
        }
        PanelWidget::Row { widgets } => {
            rsx! {div{class:"extension-action-row",for widget in widgets{{render_widget(widget,extension.clone(),signals,action)}}}}
        }
        PanelWidget::Disclosure { label, widgets } => {
            rsx! {crate::extension_panel::PanelDisclosure{label,widgets,render:Callback::new(move|widget|render_widget(widget,extension.clone(),signals,action))}}
        }
        PanelWidget::SecretInput { id, label } => {
            rsx! {label{class:"field",span{"{label}"}input{r#type:"password",autocomplete:"off",disabled:busy(),maxlength:8192,value:drafts.read().get(&id).cloned().unwrap_or_default(),oninput:move|event|{drafts.write().insert(id.clone(),event.value());}}}}
        }
        PanelWidget::Link { label, url } => {
            rsx! {a{href:url,target:"_blank",rel:"noopener noreferrer","{label}"}}
        }
        PanelWidget::SettingInput { id } => rsx! {LocalSetting{extension,id}},
        PanelWidget::Metric { label, value } => rsx! {p{"{label}: {value}"}},
        PanelWidget::CopyText { label, value } => {
            rsx! {crate::extension_panel::CopyText{label,value}}
        }
        _ => rsx! {p{class:"field-error","Unsupported local panel widget."}},
    }
}
#[cfg(target_arch = "wasm32")]
async fn drive(
    mut session: LocalSession,
    effects: Vec<Value>,
    #[cfg(target_arch = "wasm32")] mut started: Option<browser::Login>,
) -> Result<(LocalSession, Option<ExtensionPanel>), String> {
    let mut pending = VecDeque::from(effects);
    let mut panel = None;
    for _ in 0..128 {
        let Some(effect) = pending.pop_front() else {
            return Ok((session, panel));
        };
        let input = match effect["type"].as_str() {
            Some("panel") => {
                panel = Some(
                    serde_json::from_value(effect["panel"].clone())
                        .map_err(|_| "Invalid local panel.")?,
                );
                continue;
            }
            Some("http") => session.http(&effect).await,
            Some("browser" | "browser_module") => {
                #[cfg(not(target_arch = "wasm32"))]
                let response = async {
                    let url = acheflow_client::transport::allowed_url(
                        effect["url"].as_str().ok_or("Missing browser URL.")?,
                        &session.manifest,
                        false,
                    )?;
                    let callback = acheflow_client::browser::Callback::bind().await?;
                    callback.open(url.as_str())?;
                    callback.receive().await
                }
                .await;
                #[cfg(target_arch = "wasm32")]
                let response = match started.take() {
                    Some(login) => login.finish().await,
                    None => Err("Start account connection with its button.".into()),
                };
                match response {
                    Ok(mut payload) => {
                        session.capture(&mut payload, &effect["capture"])?;
                        json!({"type":"local_browser","payload":payload})
                    }
                    Err(_) => {
                        json!({"type":"local_browser","payload":null,"error":"Browser authorization interrupted."})
                    }
                }
            }
            Some("status" | "failure") => continue,
            _ => return Err("Unsupported local effect.".into()),
        };
        pending.extend(session.step(input, &environment(), now())?);
    }
    Err("Local panel event queue exceeded.".into())
}
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native::drive;
