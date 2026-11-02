use crate::{
    control::{ControlHost, now_ms},
    twitch_auth::{self, Credentials},
};
use acheflow_control::extensions::{Guest, GuestManifest};
use acheflow_media_api::control::ExtensionStatus;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Outbound {
    pub channel: String,
    pub text: String,
    pub result: oneshot::Sender<Result<(), String>>,
}
pub struct Running {
    pub cancel: CancellationToken,
    pub sender: mpsc::Sender<Outbound>,
    pub join: tokio::task::JoinHandle<()>,
}
#[cfg(test)]
fn bundled(id: &str) -> Result<(&'static [u8], GuestManifest), String> {
    let (module, manifest) = match id {
        "twitch" => (
            include_bytes!("../../../extensions/twitch/twitch.wat").as_slice(),
            include_str!("../../../extensions/twitch/manifest.json"),
        ),
        "sample" => (
            include_bytes!("../../../extensions/sample/sample.wat").as_slice(),
            include_str!("../../../extensions/sample/manifest.json"),
        ),
        _ => return Err("not_found: extension".into()),
    };
    let manifest: GuestManifest =
        serde_json::from_str(manifest).map_err(|_| "invalid bundled manifest")?;
    manifest.validate()?;
    Ok((module, manifest))
}
pub fn failed(host: &ControlHost, owner: &str, id: &str, error: String) {
    host.extension_status(
        owner,
        ExtensionStatus {
            id: id.into(),
            name: if id == "twitch" {
                "Twitch"
            } else {
                "Sample extension"
            }
            .into(),
            state: "failed".into(),
            error: Some(error),
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
    if id != "twitch" {
        return crate::guest_host::set_enabled(host, owner, id, enabled).await;
    }
    let extension = host.installed.get(id).ok_or("not_found: extension")?;
    let module = extension.module.as_slice();
    let manifest = extension.manifest.clone();
    let mut task = host.twitch.lock().await;
    if let Some(old) = task.take() {
        old.cancel.cancel();
        let _ = old.join.await;
    }
    if !enabled {
        if let Some(pending) = host.auth_cancel.lock().unwrap().take() {
            pending.cancel();
        }
        host.services.lock().unwrap().authorization = None;
        host.extension_status(
            owner,
            ExtensionStatus {
                id: id.into(),
                name: manifest.name,
                state: "stopped".into(),
                capabilities: manifest.capabilities,
                ..Default::default()
            },
        );
        return Ok(());
    }
    let credentials = host
        .credentials
        .read()
        .await
        .clone()
        .ok_or("unavailable: connect a Twitch account first")?;
    let guest = Arc::new(Mutex::new(Guest::load(
        module,
        manifest.capabilities.clone(),
    )?));
    let cancel = host.runtime.shutdown.child_token();
    let (sender, receiver) = mpsc::channel(512);
    let owner = owner.to_owned();
    let task_host = host.clone();
    let task_cancel = cancel.clone();
    host.extension_status(
        &owner,
        ExtensionStatus {
            id: id.into(),
            name: manifest.name.clone(),
            state: "connecting".into(),
            capabilities: manifest.capabilities.clone(),
            account: Some(credentials.login.clone()),
            ..Default::default()
        },
    );
    let join = tokio::spawn(async move {
        let result = tokio::select! {_=task_cancel.cancelled()=>return,result=run(&task_host,&owner,guest,&manifest,credentials,receiver)=>result};
        if let Err(error) = result {
            failed(&task_host, &owner, "twitch", error);
        }
    });
    *task = Some(Running {
        cancel,
        sender,
        join,
    });
    Ok(())
}
async fn invoke(guest: &Arc<Mutex<Guest>>, input: Value) -> Result<Vec<Value>, String> {
    let guest = guest.clone();
    tokio::task::spawn_blocking(move || {
        guest
            .lock()
            .map_err(|_| "extension instance failed")?
            .call(input)
    })
    .await
    .map_err(|_| "extension worker failed")?
}
fn input(kind: &str, credentials: &Credentials) -> Value {
    json!({"type":kind,"now_ms":now_ms(),"config":{"channel_id":credentials.user_id,"user_id":credentials.user_id}})
}
fn allowed_url(raw: &str, manifest: &GuestManifest, websocket: bool) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "invalid extension service URL")?;
    let service = url.origin().ascii_serialization();
    if !manifest.services.contains(&service)
        || url.scheme() != if websocket { "wss" } else { "https" }
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("forbidden: extension service is not granted".into());
    }
    Ok(url)
}
async fn http_effect(
    http: &reqwest::Client,
    effect: &Value,
    manifest: &GuestManifest,
    credentials: &Credentials,
) -> Result<Value, String> {
    let url = allowed_url(
        effect["url"].as_str().ok_or("missing service URL")?,
        manifest,
        false,
    )?;
    let method = match effect["method"].as_str() {
        Some("GET") => reqwest::Method::GET,
        Some("POST") => reqwest::Method::POST,
        _ => return Err("unsupported extension HTTP method".into()),
    };
    let mut request = http
        .request(method, url)
        .bearer_auth(&credentials.access_token)
        .header("Client-Id", &credentials.client_id);
    if let Some(query) = effect["query"].as_object() {
        request = request.query(&query);
    }
    if !effect["body"].is_null() {
        request = request.json(&effect["body"]);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "outcome_unknown: platform request interrupted")?;
    if response.status().as_u16() == 429 {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                response
                    .headers()
                    .get("ratelimit-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|at| at.saturating_sub(now_ms() / 1000))
            })
            .unwrap_or(30)
            .clamp(1, 3600);
        return Err(format!("rate_limited:{retry}"));
    }
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "forbidden: reconnect the Twitch account",
            _ => "unavailable: Twitch request failed",
        }
        .into());
    }
    twitch_auth::bounded_json(response).await
}
async fn run(
    host: &Arc<ControlHost>,
    owner: &str,
    guest: Arc<Mutex<Guest>>,
    manifest: &GuestManifest,
    mut credentials: Credentials,
    mut outgoing: mpsc::Receiver<Outbound>,
) -> Result<(), String> {
    let http = twitch_auth::client()?;
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut socket = None;
    let mut pending: std::collections::VecDeque<Value> =
        invoke(&guest, input("start", &credentials)).await?.into();
    let mut failures = 0u32;
    let mut next_poll_ms = 0u64;
    let mut resuming = false;
    let mut heartbeat = Duration::from_secs(75);
    loop {
        if !host.owns(owner) {
            return Ok(());
        }
        if pending.len() > 512 {
            return Err("extension event queue exceeded".into());
        }
        while let Some(effect) = pending.pop_front() {
            match effect["type"].as_str() {
                Some("websocket") => {
                    let url = allowed_url(
                        effect["url"].as_str().ok_or("missing WebSocket URL")?,
                        manifest,
                        true,
                    )?;
                    let config =
                        tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                            .max_message_size(Some(1024 * 1024))
                            .max_frame_size(Some(1024 * 1024));
                    let connected = tokio::time::timeout(
                        Duration::from_secs(10),
                        tokio_tungstenite::connect_async_with_config(
                            url.as_str(),
                            Some(config),
                            false,
                        ),
                    )
                    .await;
                    let (next, _) = match connected {
                        Ok(Ok(connected)) => connected,
                        _ => {
                            failures = failures.saturating_add(1);
                            host.extension_status(
                                owner,
                                ExtensionStatus {
                                    id: manifest.id.clone(),
                                    name: manifest.name.clone(),
                                    state: "reconnecting".into(),
                                    error: Some("Twitch chat connection failed; retrying".into()),
                                    capabilities: manifest.capabilities.clone(),
                                    account: Some(credentials.login.clone()),
                                },
                            );
                            tokio::time::sleep(Duration::from_secs(
                                (1u64 << failures.min(5)).min(30),
                            ))
                            .await;
                            pending.push_front(effect);
                            continue;
                        }
                    };
                    resuming = effect["resume"] == true;
                    socket = Some(next);
                    host.extension_status(
                        owner,
                        ExtensionStatus {
                            id: manifest.id.clone(),
                            name: manifest.name.clone(),
                            state: "running".into(),
                            capabilities: manifest.capabilities.clone(),
                            account: Some(credentials.login.clone()),
                            ..Default::default()
                        },
                    );
                }
                Some("heartbeat") => {
                    heartbeat = Duration::from_secs(
                        effect["seconds"].as_u64().unwrap_or(10).clamp(5, 600) + 5,
                    )
                }
                Some("http") => {
                    if effect["id"] == "viewers" && now_ms() < next_poll_ms {
                        continue;
                    }
                    match http_effect(&http, &effect, manifest, &credentials).await {
                        Ok(payload) => {
                            let mut response = input("http_result", &credentials);
                            response["id"] = effect["id"].clone();
                            response["payload"] = payload;
                            pending.extend(invoke(&guest, response).await?);
                            failures = 0;
                        }
                        Err(error) => {
                            failures = failures.saturating_add(1);
                            let error = if let Some(seconds) = error
                                .strip_prefix("rate_limited:")
                                .and_then(|s| s.parse::<u64>().ok())
                            {
                                next_poll_ms = now_ms().saturating_add(seconds * 1000);
                                "rate_limited: Twitch requested a pause".to_owned()
                            } else {
                                error
                            };
                            if effect["id"] == "subscribe" || error.starts_with("forbidden") {
                                return Err(error);
                            }
                            host.extension_status(
                                owner,
                                ExtensionStatus {
                                    id: manifest.id.clone(),
                                    name: manifest.name.clone(),
                                    state: "reconnecting".into(),
                                    error: Some(error),
                                    capabilities: manifest.capabilities.clone(),
                                    account: Some(credentials.login.clone()),
                                },
                            );
                        }
                    }
                }
                _ => host.publish(owner, effect).await?,
            }
        }
        tokio::select! {
            _=tick.tick()=>{
                if credentials.expires_at_ms<now_ms().saturating_add(60000) {
                    credentials=twitch_auth::refresh(&http,&credentials).await?;
                    *host.credentials.write().await=Some(credentials.clone());
                }
                pending.extend(invoke(&guest,input("tick",&credentials)).await?);
            }
            outbound=outgoing.recv()=>{
                let Some(outbound)=outbound else {return Ok(());};
                let mut event=input("send",&credentials);event["channel_id"]=json!(outbound.channel);event["text"]=json!(outbound.text);
                let result=async {
                    let effects=invoke(&guest,event).await?;
                    let request=effects.first().filter(|e|e["type"]=="http").ok_or("guest did not produce send request")?;
                    let payload=http_effect(&http,request,manifest,&credentials).await?;
                    let mut response=input("http_result",&credentials);response["id"]=json!("send");response["payload"]=payload;
                    let effects=invoke(&guest,response).await?;
                    if effects.first().is_some_and(|e|e["type"]=="delivery" && e["sent"]==true) {Ok(())}else {Err("unavailable: Twitch rejected the message".into())}
                }.await;
                let _=outbound.result.send(result);
            }
            message=async {match socket.as_mut(){Some(socket)=>tokio::time::timeout(heartbeat,socket.next()).await.ok().flatten(),None=>None}}=>{
                match message {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))=>{
                        if text.len()>1024*1024 {return Err("Twitch event exceeds 1 MiB".into());}
                        let payload:Value=serde_json::from_str(&text).map_err(|_|"invalid Twitch event")?;
                        let mut event=input("websocket",&credentials);event["resuming"]=json!(resuming);event["payload"]=payload;
                        pending.extend(invoke(&guest,event).await?);
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(bytes)))=>{if let Some(socket)=socket.as_mut(){socket.send(tokio_tungstenite::tungstenite::Message::Pong(bytes)).await.map_err(|_|"Twitch ping response failed")?;}},
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))|Some(Err(_))|None=>{
                        socket=None;
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        pending.extend(invoke(&guest,input("start",&credentials)).await?);
                    }
                    _=>{},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_can_only_be_used_for_granted_https_origins() {
        let (_, manifest) = bundled("twitch").unwrap();
        assert!(allowed_url("https://api.twitch.tv/helix/streams", &manifest, false).is_ok());
        for url in [
            "http://api.twitch.tv/helix/streams",
            "https://api.twitch.tv.evil.test/",
            "https://api.twitch.tv@evil.test/",
            "https://api.twitch.tv:444/",
        ] {
            assert!(allowed_url(url, &manifest, false).is_err());
        }
    }
}
