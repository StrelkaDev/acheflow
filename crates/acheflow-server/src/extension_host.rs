//! One generic WASM lifecycle for every installed extension.
use crate::control::{ControlHost, now_ms};
use acheflow_control::extensions::Guest;
use acheflow_media_api::control::{ControlAction, ControlRequest, ExtensionStatus};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct Running {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
    owner: String,
    instance: String,
    forward: tokio::sync::mpsc::Sender<Value>,
}
impl Running {
    pub(crate) fn forward(&self, owner: &str, instance: &str, input: Value) -> Result<(), String> {
        if self.owner != owner || self.instance != instance || self.cancel.is_cancelled() {
            return Err("unavailable: forwarding instance changed".into());
        }
        self.forward
            .try_send(input)
            .map_err(|_| "unavailable: forwarding inbox is full or stopped".into())
    }
}
pub fn failed(host: &ControlHost, owner: &str, id: &str, error: String) {
    let Some(extension) = host.installed.get(id) else {
        return;
    };
    {
        let mut state = host.services.lock().unwrap();
        state.integrations.disconnect(id);
        state.analytics.disconnect(id);
        state.interaction.disconnect(id);
    }
    host.extension_status(
        owner,
        ExtensionStatus {
            id: id.into(),
            name: extension.manifest.name.clone(),
            state: "failed".into(),
            error: Some(error),
            capabilities: extension.manifest.capabilities.clone(),
            settings: extension
                .manifest
                .settings
                .iter()
                .filter(|s| !s.secret)
                .cloned()
                .collect(),
            ..Default::default()
        },
    );
}
pub async fn set_enabled(
    host: Arc<ControlHost>,
    owner: &str,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    let extension = host.installed.get(id).ok_or("not_found: extension")?;
    let manifest = extension.manifest.clone();
    manifest.validate()?;
    if manifest.execution_side == acheflow_extension_api::integration::ExecutionSide::Client {
        return Err("client integrations cannot be installed or activated on the server".into());
    }
    let mut running = host.guests.lock().await;
    host.disconnect_extension(owner, id);
    if let Some(old) = running.remove(id) {
        old.cancel.cancel();
        let _ = old.join.await;
    }
    let status = ExtensionStatus {
        id: id.into(),
        name: manifest.name.clone(),
        capabilities: manifest.capabilities.clone(),
        settings: manifest
            .settings
            .iter()
            .filter(|s| !s.secret)
            .cloned()
            .collect(),
        state: if enabled { "running" } else { "stopped" }.into(),
        ..Default::default()
    };
    host.extension_status(owner, status.clone());
    if !enabled {
        return Ok(());
    }
    let guest = Arc::new(Mutex::new(Guest::load(
        &extension.module,
        manifest.capabilities.clone(),
    )?));
    let instance = acheflow_media_api::workspace::new_token();
    host.services
        .lock()
        .unwrap()
        .instances
        .insert(id.into(), instance.clone());
    let cancel = host.runtime.shutdown.child_token();
    let task_cancel = cancel.clone();
    let task_host = host.clone();
    let owner = owner.to_owned();
    let id = id.to_owned();
    let installed_id = manifest.id.clone();
    let instance_id = instance.clone();
    let owner_id = owner.clone();
    let (forward, inbox) = tokio::sync::mpsc::channel(32);
    let join = tokio::spawn(async move {
        let result = tokio::select! {_=task_cancel.cancelled()=>return,result=run(&task_host,&owner,&id,&instance,guest,&manifest,inbox)=>result};
        if let Err(error) = result
            && task_host.extension_is_live(&owner, &id, &instance)
        {
            failed(&task_host, &owner, &id, error);
            task_host.disconnect_extension(&owner, &id);
        }
    });
    running.insert(
        installed_id,
        Running {
            cancel,
            join,
            owner: owner_id,
            instance: instance_id,
            forward,
        },
    );
    Ok(())
}
async fn invoke(guest: &Arc<Mutex<Guest>>, input: Value) -> Result<Vec<Value>, String> {
    let guest = guest.clone();
    tokio::task::spawn_blocking(move || {
        guest
            .lock()
            .map_err(|_| "extension instance failed".to_owned())?
            .call(input)
    })
    .await
    .map_err(|_| "extension worker failed".to_owned())?
}
async fn run(
    host: &Arc<ControlHost>,
    owner: &str,
    id: &str,
    instance: &str,
    guest: Arc<Mutex<Guest>>,
    manifest: &acheflow_control::extensions::GuestManifest,
    mut forward: tokio::sync::mpsc::Receiver<Value>,
) -> Result<(), String> {
    let mut events = host.events.subscribe();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let key = (owner.to_owned(), id.to_owned());
    let mut storage = host
        .extension_storage
        .lock()
        .unwrap()
        .get(&key)
        .cloned()
        .unwrap_or(Value::Null);
    let mut pending = std::collections::VecDeque::from([json!({"type":"start"})]);
    loop {
        if !host.extension_is_live(owner, id, instance) {
            return Ok(());
        }
        if pending.len() > 512 {
            return Err("extension event queue exceeded".into());
        }
        if let Some(mut input) = pending.pop_front() {
            input["api_version"] = json!(acheflow_extension_api::API_VERSION);
            input["workspace_id"] = json!(owner);
            input["extension_id"] = json!(id);
            input["instance_id"] = json!(instance);
            input["now_ms"] = json!(now_ms());
            if input["request_id"].is_null() {
                input["request_id"] = json!(format!(
                    "{}:{}",
                    now_ms(),
                    acheflow_media_api::workspace::new_token()
                ));
            }
            input["config"] = serde_json::to_value(
                host.runtime
                    .workspace
                    .read()
                    .unwrap()
                    .as_ref()
                    .and_then(|w| w.extensions.get(id))
                    .map(|s| s.config.clone())
                    .unwrap_or_default(),
            )
            .unwrap();
            input["storage"] = storage.clone();
            if manifest.capabilities.iter().any(|c| c == "scene") {
                let state = host.snapshot(0, 1);
                input["state"] = json!({"workspace_id":state.workspace_id,"revision":state.revision,"scenes":state.scenes});
            }
            let depth = input["depth"].as_u64().unwrap_or(0).min(255) as u8;
            for effect in invoke(&guest, input).await? {
                if !host.extension_is_live(owner, id, instance) {
                    return Ok(());
                }
                match effect["type"].as_str() {
                    Some("http" | "websocket") => {
                        return Err("server extensions cannot perform provider I/O".into());
                    }
                    Some("storage") => {
                        if serde_json::to_vec(&effect["value"])
                            .map_err(|_| "invalid storage value")?
                            .len()
                            > 65536
                        {
                            return Err("extension storage exceeds 64 KiB".into());
                        }
                        storage = effect["value"].clone();
                        {
                            let mut saved = host.extension_storage.lock().unwrap();
                            if !saved.contains_key(&key) && saved.len() >= 256 {
                                return Err("extension storage namespace limit reached".into());
                            }
                            saved.insert(key.clone(), storage.clone());
                        }
                    }
                    Some("scene") => {
                        let request = ControlRequest {
                            workspace_id: owner.into(),
                            request_id: effect["request_id"]
                                .as_str()
                                .ok_or("scene effects require request_id")?
                                .into(),
                            expected_revision: effect["expected_revision"]
                                .as_u64()
                                .ok_or("scene effects require expected_revision")?,
                            action: ControlAction::SetScene {
                                scene_id: effect["scene_id"]
                                    .as_u64()
                                    .and_then(|id| u32::try_from(id).ok())
                                    .ok_or("invalid scene id")?,
                                spatial: serde_json::from_value(effect["spatial"].clone())
                                    .map_err(|_| "invalid extension scene")?,
                            },
                        };
                        host.apply_extension_scene(request)?;
                    }
                    Some("failure") => {
                        let message = effect["message"]
                            .as_str()
                            .ok_or("missing extension failure message")?
                            .chars()
                            .take(500)
                            .collect();
                        let mut services = host.services.lock().unwrap();
                        if let Some(status) = services.extensions.iter_mut().find(|s| s.id == id) {
                            status.error = Some(message);
                        }
                    }
                    _ => {
                        let fatal = effect["type"] == "status" && effect["state"] == "failed";
                        host.publish_from(owner, id, instance, effect, depth.saturating_add(1))
                            .await?;
                        if fatal {
                            return Err("Extension reported a fatal failure; disable and enable it to restart".into());
                        }
                    }
                }
            }
            continue;
        }
        tokio::select! {
            input=forward.recv()=>match input {Some(input)=>pending.push_back(input),None=>return Ok(())},
            _=tick.tick()=>pending.push_back(json!({"type":"tick"})),
            event=events.recv()=>match event {
                Ok(event) if event["workspace_id"]==owner && (event["target"]==id || (event["target"].is_null() && event["origin"]!=id && event["name"].as_str().is_some_and(|name|manifest.subscriptions.iter().any(|s|s==name))))=>{
                    if event["type"]!="command" || event["name"].as_str().is_some_and(|name|manifest.subscriptions.iter().any(|s|s==name)) {pending.push_back(event);}
                },Ok(_)=>{},Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>return Err("Event cursor gap; restart the extension to reconnect".into()),Err(_)=>return Ok(()),
            },
        }
    }
}
