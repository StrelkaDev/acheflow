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
}
pub async fn set_enabled(
    host: Arc<ControlHost>,
    owner: &str,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    let extension = host.installed.get(id).ok_or("not_found: extension")?;
    let manifest = extension.manifest.clone();
    // Generic guests receive no account credentials or transport grants. Platform
    // bindings explicitly provide those capabilities, as the Twitch binding does.
    if manifest
        .capabilities
        .iter()
        .any(|c| matches!(c.as_str(), "http" | "websocket"))
    {
        return Err("forbidden: this extension has no platform transport binding".into());
    }
    let mut running = host.guests.lock().await;
    if let Some(old) = running.remove(id) {
        old.cancel.cancel();
        let _ = old.join.await;
    }
    let mut status = ExtensionStatus {
        id: id.into(),
        name: manifest.name,
        capabilities: manifest.capabilities.clone(),
        state: if enabled { "running" } else { "stopped" }.into(),
        ..Default::default()
    };
    if !enabled {
        host.extension_status(owner, status);
        return Ok(());
    }
    let guest = Arc::new(Mutex::new(Guest::load(
        &extension.module,
        manifest.capabilities,
    )?));
    let cancel = host.runtime.shutdown.child_token();
    let task_cancel = cancel.clone();
    let owner = owner.to_owned();
    host.extension_status(&owner, status.clone());
    let task_host = host.clone();
    let mut events = host.events.subscribe();
    let join = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        let mut input = json!({"type":"start","now_ms":now_ms(),"request_id":format!("{}:{}",now_ms(),acheflow_media_api::workspace::new_token())});
        loop {
            if !task_host.owns(&owner) {
                return;
            }
            let depth = input["depth"].as_u64().unwrap_or(0).min(255) as u8;
            input["state"] =
                serde_json::to_value(task_host.snapshot(0, 200)).unwrap_or(Value::Null);
            let guest = guest.clone();
            let result = tokio::select! {_=task_cancel.cancelled()=>return,result=tokio::task::spawn_blocking(move ||guest.lock().map_err(|_|"guest failed".to_owned())?.call(input))=>result.map_err(|_|"extension worker failed".to_owned()).and_then(|r|r)};
            let effects = match result {
                Ok(effects) => effects,
                Err(error) => {
                    status.state = "failed".into();
                    status.error = Some(error);
                    task_host.extension_status(&owner, status);
                    return;
                }
            };
            if let Err(error) = deliver(&task_host, &owner, &status.id, effects, depth).await {
                status.error = Some(error);
                task_host.extension_status(&owner, status.clone());
            }
            input = tokio::select! {
                _=task_cancel.cancelled()=>return,
                _=tick.tick()=>json!({"type":"tick","now_ms":now_ms(),"request_id":format!("{}:{}",now_ms(),acheflow_media_api::workspace::new_token())}),
                event=async {loop {match events.recv().await {Ok(event) if event["origin"]!=status.id && event["depth"].as_u64().unwrap_or(4)<4 && (event["target"].is_null() || event["target"]==status.id)=>break Ok(event),Ok(_)=>continue,Err(error)=>break Err(error)}}}=>match event {Ok(event)=>event,Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{status.error=Some("Event cursor gap; refresh the state snapshot".into());task_host.extension_status(&owner,status.clone());json!({"type":"gap"})},Err(_)=>return},
            };
        }
    });
    running.insert(id.into(), Running { cancel, join });
    Ok(())
}
async fn deliver(
    host: &Arc<ControlHost>,
    owner: &str,
    id: &str,
    effects: Vec<Value>,
    depth: u8,
) -> Result<(), String> {
    for effect in effects {
        if effect["type"] == "scene" {
            let scene_id = effect["scene_id"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("invalid scene id")?;
            let spatial = serde_json::from_value(effect["spatial"].clone())
                .map_err(|_| "invalid extension scene")?;
            let request_id = effect["request_id"]
                .as_str()
                .ok_or("scene effects require a stable request_id")?
                .to_owned();
            // A busy host returns a visible conflict without creating an
            // unbounded task queue or blocking a guest stop operation.
            let request = ControlRequest {
                workspace_id: owner.into(),
                request_id,
                expected_revision: effect["expected_revision"]
                    .as_u64()
                    .ok_or("scene effects require expected_revision from their state snapshot")?,
                action: ControlAction::SetScene { scene_id, spatial },
            };
            host.apply_extension_scene(request)?;
        } else {
            host.publish_from(owner, id, effect, depth.saturating_add(1))
                .await?;
        }
    }
    Ok(())
}
