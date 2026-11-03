//! One generic WASM lifecycle for every installed extension.
use crate::{
    control::{ControlHost, now_ms},
    extension_transport,
};
use acheflow_control::extensions::Guest;
use acheflow_media_api::control::{ControlAction, ControlRequest, ExtensionStatus};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_util::sync::CancellationToken;

pub struct Running {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
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
            settings: extension.manifest.settings.clone(),
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
        settings: manifest.settings.clone(),
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
    let join = tokio::spawn(async move {
        let result = tokio::select! {_=task_cancel.cancelled()=>return,result=run(&task_host,&owner,&id,&instance,guest,&manifest)=>result};
        if let Err(error) = result
            && task_host.extension_is_live(&owner, &id, &instance)
        {
            failed(&task_host, &owner, &id, error);
            task_host.disconnect_extension(&owner, &id);
        }
    });
    running.insert(installed_id, Running { cancel, join });
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
) -> Result<(), String> {
    let mut events = host.events.subscribe();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut socket: Option<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    > = None;
    let key = (owner.to_owned(), id.to_owned());
    let mut secrets = host
        .secrets
        .lock()
        .await
        .get(&key)
        .cloned()
        .unwrap_or_default();
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
            input["secrets"] = json!(secrets.keys().collect::<Vec<_>>());
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
                    Some("http") => {
                        let mut result=extension_transport::http(&effect,manifest,&mut secrets).await.unwrap_or_else(|error|json!({"type":"http_result","id":effect["id"],"status":0,"error":error,"payload":null}));
                        if !host.extension_is_live(owner, id, instance) {
                            return Ok(());
                        }
                        {
                            let mut saved = host.secrets.lock().await;
                            if !saved.contains_key(&key) && saved.len() >= 256 {
                                return Err("extension credential namespace limit reached".into());
                            }
                            saved.insert(key.clone(), secrets.clone());
                        }
                        result["depth"] = json!(depth);
                        pending.push_back(result);
                    }
                    Some("websocket") => {
                        if let Some(payload) = effect.get("send") {
                            let payload = extension_transport::resolve(payload, &secrets)?;
                            let text = serde_json::to_string(&payload)
                                .map_err(|_| "invalid WebSocket JSON")?;
                            let connection = socket.as_mut().ok_or("WebSocket is not connected")?;
                            tokio::time::timeout(
                                Duration::from_secs(10),
                                connection.send(Message::Text(text.into())),
                            )
                            .await
                            .map_err(|_| "WebSocket send timed out")?
                            .map_err(|_| "WebSocket send failed")?;
                            pending.push_back(
                                json!({"type":"websocket_sent","id":effect["id"],"depth":depth}),
                            );
                            continue;
                        }
                        if effect["close"] == true {
                            socket = None;
                            continue;
                        }
                        let url = extension_transport::allowed_url(
                            effect["url"].as_str().ok_or("missing WebSocket URL")?,
                            manifest,
                            true,
                        )?;
                        let config =
                            tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                                .max_message_size(Some(1024 * 1024))
                                .max_frame_size(Some(1024 * 1024));
                        let mut request = url
                            .as_str()
                            .into_client_request()
                            .map_err(|_| "invalid WebSocket request")?;
                        let headers = extension_transport::resolve(&effect["headers"], &secrets)?;
                        if let Some(headers) = headers.as_object() {
                            if headers.len() > 32 {
                                return Err("too many request headers".into());
                            }
                            for (name, value) in headers {
                                extension_transport::validate_header(name)?;
                                let name =
                                    tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(
                                        name.as_bytes(),
                                    )
                                    .map_err(|_| "invalid WebSocket header")?;
                                let value =
                                    tokio_tungstenite::tungstenite::http::HeaderValue::from_str(
                                        value.as_str().ok_or("invalid WebSocket header")?,
                                    )
                                    .map_err(|_| "invalid WebSocket header")?;
                                request.headers_mut().insert(name, value);
                            }
                        }
                        match tokio::time::timeout(Duration::from_secs(10),tokio_tungstenite::connect_async_with_config(request,Some(config),true)).await {
                            Ok(Ok((connection,_)))=>{socket=Some(connection);pending.push_back(json!({"type":"websocket_open","depth":depth}));},
                            _=>pending.push_back(json!({"type":"websocket_closed","error":"WebSocket connection failed","depth":depth})),
                        }
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
            _=tick.tick()=>pending.push_back(json!({"type":"tick"})),
            event=events.recv()=>match event {
                Ok(event) if event["workspace_id"]==owner && (event["target"]==id || (event["target"].is_null() && event["origin"]!=id && event["name"].as_str().is_some_and(|name|manifest.subscriptions.iter().any(|s|s==name))))=>{
                    if event["type"]!="command" || event["name"].as_str().is_some_and(|name|manifest.subscriptions.iter().any(|s|s==name)) {pending.push_back(event);}
                },Ok(_)=>{},Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>return Err("Event cursor gap; restart the extension to reconnect".into()),Err(_)=>return Ok(()),
            },
            message=async {if let Some(socket)=socket.as_mut(){socket.next().await}else{std::future::pending().await}}=>{
                match message {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))=>match serde_json::from_str::<Value>(&text){Ok(payload)=>pending.push_back(json!({"type":"websocket","payload":payload})),Err(_)=>return Err("invalid WebSocket JSON".into())},
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data)))=>{if let Some(socket)=socket.as_mut(){tokio::time::timeout(Duration::from_secs(10),socket.send(Message::Pong(data))).await.map_err(|_|"WebSocket ping timed out")?.map_err(|_|"WebSocket ping failed")?;}},
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))|Some(Err(_))|None=>{socket=None;pending.push_back(json!({"type":"websocket_closed"}));},_=>{}
                }
            }
        }
    }
}
