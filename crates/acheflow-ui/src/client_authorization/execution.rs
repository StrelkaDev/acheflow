//! Persistent native execution for enabled, locally authorized guest installations.
use super::*;
use acheflow_client::{
    bridge::ExecutorConnection,
    runtime::{Identity, ProviderRuntime},
};
use acheflow_extension_api::client_execution::*;
use acheflow_media_api::control::{ControlAction, ControlRequest};

#[component]
pub(crate) fn ClientExecutionPanel(id: String) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let sessions = use_context::<Signal<Sessions>>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let mut panel = use_signal(|| None::<ExtensionPanel>);
    let mut error = use_signal(String::new);
    let mut state = use_signal(|| "Waiting for local authorization".to_owned());
    let mut actions = use_signal(VecDeque::<Value>::new);
    let mut generation = use_signal(now);
    let permissions = use_signal(std::collections::BTreeSet::<String>::new);
    let actor = use_hook(|| format!("studio-{}", &crate::project::new_token()[..32]));
    let owner = use_memo({
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
    use_resource({
        let id = id.clone();
        move || {
            let owner = owner();
            let id = id.clone();
            let catalog = catalog.clone();
            let actor = actor.clone();
            panel.set(None);
            error.set(String::new());
            actions.write().clear();
            async move {
                let mut failures = 0u8;
                loop {
                    let prepared = (|| {
                        let sessions = sessions.peek();
                        let Some(local) = sessions.get(&session_key(&owner, &id)) else {
                            return Ok(None);
                        };
                        let Some(revision) = local.credential_revision() else {
                            return Ok(None);
                        };
                        let installed = &catalog.installed[&id];
                        let next = (*generation.peek()).saturating_add(1).max(now());
                        generation.set(next);
                        let identity = Identity {
                            workspace_id: owner.0.clone(),
                            client_id: actor.clone(),
                            generation: next,
                            connection: Connection {
                                id: "pending".into(),
                                provider: id.clone(),
                                account_id: local.account_id(),
                                channel_id: "pending".into(),
                                capabilities: Default::default(),
                            },
                        };
                        let mut runtime = ProviderRuntime::new(
                            installed.manifest.clone(),
                            &installed.module,
                            identity,
                            owner.2.clone(),
                        )?;
                        local.install(&mut runtime)?;
                        Ok::<_, String>(Some((revision, runtime)))
                    })();
                    match prepared {
                        Ok(Some((revision, mut runtime))) => {
                            state.set("Finding live channels…".into());
                            error.set(String::new());
                            let result = run(
                                &mut runtime,
                                &owner,
                                &id,
                                revision,
                                ExecutionSignals {
                                    client,
                                    sessions,
                                    panel,
                                    state,
                                    actions,
                                    permissions,
                                },
                            )
                            .await;
                            generation.set(runtime.identity().generation);
                            panel.set(None);
                            if let Err(reason) = result {
                                failures = failures.saturating_add(1);
                                crate::diagnostics::record(
                                    "error",
                                    &format!("extension:{id}"),
                                    &reason,
                                );
                                error.set(reason);
                                state.set("Live events disconnected".into());
                            } else {
                                failures = 0;
                            }
                        }
                        Ok(None) => {
                            failures = 0;
                            state.set("Waiting for local authorization".into());
                        }
                        Err(reason) => {
                            failures = failures.saturating_add(1);
                            error.set(reason);
                        }
                    }
                    crate::scene_preview::pause(if failures == 0 {
                        2000
                    } else {
                        crate::live_client::polling_delay(false, failures, 2000)
                    })
                    .await;
                }
            }
        }
    });
    let action = Callback::new(move |value: Value| {
        if actions.peek().len() < 32 {
            actions.write().push_back(value);
        }
    });
    rsx! {div{class:"local-execution-panel",
        p{class:"form-help",role:"status","{state}"}
        if !error().is_empty(){p{class:"field-error",role:"alert","{error}"}}
        if let Some(panel)=panel.read().as_ref(){
            h3{"{panel.title}"}
            for widget in panel.widgets.clone() {{widget_view(widget,action)}}
        }
    }}
}

struct Registration {
    endpoint: String,
    workspace: String,
    actor: String,
    generation: Option<u64>,
}
impl Registration {
    fn bridge(&self) -> Result<ExecutorConnection, String> {
        ExecutorConnection::studio(
            self.endpoint.clone(),
            self.workspace.clone(),
            self.actor.clone(),
        )
    }
    async fn withdraw(&mut self) -> Result<(), String> {
        if let Some(generation) = self.generation.take() {
            self.bridge()?.disconnect(generation).await?;
        }
        Ok(())
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(generation) = self.generation.take()
            && let Ok(bridge) = self.bridge()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            // Scope/task cancellation drops provider I/O immediately; revoke server
            // authority independently. A failed unregister expires under the TTL.
            handle.spawn(async move {
                let _ = bridge.disconnect(generation).await;
            });
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ExecutionSignals {
    pub(super) client: crate::control_client::ControlClient,
    pub(super) sessions: Signal<Sessions>,
    pub(super) panel: Signal<Option<ExtensionPanel>>,
    pub(super) state: Signal<String>,
    pub(super) actions: Signal<VecDeque<Value>>,
    pub(super) permissions: Signal<std::collections::BTreeSet<String>>,
}
pub(super) async fn run(
    runtime: &mut ProviderRuntime,
    owner: &Owner,
    id: &str,
    revision: u64,
    signals: ExecutionSignals,
) -> Result<(), String> {
    let ExecutionSignals {
        client,
        sessions,
        mut panel,
        mut state,
        mut actions,
        permissions,
    } = signals;
    let workspace = client.project.peek().clone();
    let endpoint = crate::media::server_url(&workspace, "/api/control/executor")?;
    let mut registration = Registration {
        endpoint,
        workspace: owner.0.clone(),
        actor: runtime.identity().client_id.clone(),
        generation: None,
    };
    let bridge = registration.bridge()?;
    let mut output = runtime.turn(json!({"type":"start"}), now()).await?;
    let mut tick = 0;
    let mut heartbeat = 0;
    loop {
        for effect in &output {
            match effect["type"].as_str() {
                Some("panel") => panel.set(Some(
                    serde_json::from_value(effect["panel"].clone())
                        .map_err(|_| "Invalid local provider panel")?,
                )),
                Some("status") => {
                    let value = effect["state"].as_str().unwrap_or("unknown");
                    let message = effect["error"].as_str().unwrap_or(value);
                    if state.peek().as_str() != message {
                        crate::diagnostics::record(
                            if matches!(value, "failed" | "needs_authorization") {
                                "error"
                            } else if value == "reconnecting" {
                                "warn"
                            } else {
                                "info"
                            },
                            &format!("extension:{id}"),
                            message,
                        );
                    }
                    if let Some(error) = effect["error"].as_str() {
                        state.set(error.into());
                    } else if let Some(value) = effect["state"].as_str() {
                        state.set(value.into());
                    }
                }
                Some("failure") => {
                    crate::diagnostics::record(
                        "error",
                        &format!("extension:{id}"),
                        effect["message"]
                            .as_str()
                            .unwrap_or("Provider request failed"),
                    );
                    state.set(
                        effect["message"]
                            .as_str()
                            .unwrap_or("Provider request failed")
                            .into(),
                    );
                }
                Some("channel") if effect["reset"] == true => registration.withdraw().await?,
                Some("channel") => {
                    let channel = &effect["channel"];
                    let current = &runtime.identity().connection;
                    let capabilities = channel["capabilities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| serde_json::from_value::<Capability>(v.clone()).ok())
                        .collect();
                    let mut connection = Connection {
                        id: current.id.clone(),
                        provider: id.into(),
                        account_id: current.account_id.clone(),
                        channel_id: channel["id"]
                            .as_str()
                            .ok_or("Invalid discovered channel")?
                            .into(),
                        capabilities,
                    };
                    if current.channel_id != connection.channel_id
                        || current.capabilities != connection.capabilities
                        || registration.generation.is_none()
                    {
                        registration.withdraw().await?;
                        connection.id = crate::project::new_token();
                        runtime.select_connection(connection)?;
                        authorize(
                            client,
                            &workspace,
                            &runtime.identity().connection,
                            permissions,
                        )
                        .await?;
                        bridge.heartbeat(runtime).await?;
                        registration.generation = Some(runtime.identity().generation);
                        heartbeat = now() + 10_000;
                    }
                }
                _ => {}
            }
        }
        if registration.generation.is_some() {
            bridge.publish(runtime, &output).await?;
        }
        output.clear();
        let current = now();
        if current >= tick {
            if let Some(local) = sessions.peek().get(&session_key(owner, id))
                && local.credential_revision() != Some(revision)
            {
                return Ok(());
            }
            output = runtime.turn(json!({"type":"tick"}), current).await?;
            tick = current + 1000;
            if registration.generation.is_some() {
                if current >= heartbeat {
                    bridge.heartbeat(runtime).await?;
                    heartbeat = now() + 10_000;
                }
                bridge.execute_pending(runtime, current).await?;
            }
            continue;
        }
        let action = actions.write().pop_front();
        if let Some(action) = action {
            output = runtime.turn(action, current).await?;
            continue;
        }
        output = tokio::select! {
            result=runtime.receive(current)=>result?,
            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>vec![],
        };
    }
}

async fn authorize(
    client: crate::control_client::ControlClient,
    workspace: &crate::project::Workspace,
    connection: &Connection,
    mut permissions: Signal<std::collections::BTreeSet<String>>,
) -> Result<(), String> {
    let snapshot = crate::control_client::fetch(workspace, None).await?;
    if snapshot.workspace_id.as_deref() != Some(workspace.id.as_str()) {
        return Err("Apply this workspace to its server to connect live chat.".into());
    }
    let key = serde_json::to_string(&(
        workspace.id.as_str(),
        workspace.server_url.as_str(),
        connection.account_id.as_str(),
        connection.channel_id.as_str(),
    ))
    .map_err(|_| "Invalid permission identity")?;
    let mut grants = snapshot.client_capabilities.clone();
    if !grants.iter().any(|g| {
        g.provider == connection.provider
            && g.account_id == connection.account_id
            && g.channel_id == connection.channel_id
    }) {
        if permissions.peek().contains(&key) {
            return Err("This channel's workspace permissions were revoked. Re-enable the local integration to reconnect.".into());
        }
        // Enabling this local integration and its guest-declared functions is the
        // Studio operator's grant. Existing (possibly narrower) policy is preserved.
        grants.push(CapabilityGrant {
            provider: connection.provider.clone(),
            account_id: connection.account_id.clone(),
            channel_id: connection.channel_id.clone(),
            capabilities: connection.capabilities.clone(),
        });
        let request = ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:{}", now(), crate::project::new_token()),
            expected_revision: snapshot.revision,
            action: ControlAction::SetClientCapabilities { grants },
        };
        let snapshot = crate::control_client::fetch(workspace, Some(request)).await?;
        client.accept(snapshot);
    }
    permissions.write().insert(key);
    Ok(())
}

fn widget_view(widget: PanelWidget, action: Callback<Value>) -> Element {
    match widget {
        PanelWidget::Label { text } => rsx! {p{class:"form-help","{text}"}},
        PanelWidget::CopyText { label, value } => {
            rsx! {crate::extension_panel::CopyText{label,value}}
        }
        PanelWidget::Button { id, label, .. } => {
            rsx! {button{class:"local-extension-action",onclick:move|_|action.call(json!({"type":"action","action_id":id,"value":""})),"{label}"}}
        }
        PanelWidget::Select {
            id,
            label,
            value,
            options,
        } => {
            rsx! {crate::extension_panel::PanelSelect{label,value,options,onselect:move|value|action.call(json!({"type":"action","action_id":id,"value":value}))}}
        }
        PanelWidget::Row { widgets } => {
            rsx! {div{class:"extension-action-row",for widget in widgets{{widget_view(widget,action)}}}}
        }
        PanelWidget::Disclosure { label, widgets } => {
            rsx! {crate::extension_panel::PanelDisclosure{label,widgets,render:Callback::new(move|w|widget_view(w,action))}}
        }
        _ => rsx! {p{class:"form-help","Unsupported live provider control"}},
    }
}
