use super::*;
use crate::local::LocalSession;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
// Production still requires HTTPS. This transport terminates only the synthetic
// fixture's HTTPS URL on its same-origin loopback listener.
struct LoopbackHttp;
impl ActionHttp for LoopbackHttp {
    fn request<'a>(
        &'a mut self,
        effect: &'a Value,
        manifest: &'a GuestManifest,
        secrets: &'a mut Secrets,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + 'a>> {
        Box::pin(async move {
            let mut url = transport::allowed_url(effect["url"].as_str().unwrap(), manifest, false)?;
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            url.set_scheme("http").unwrap();
            let headers = transport::resolve(&effect["headers"], secrets)?;
            let response = reqwest::Client::new()
                .post(url)
                .header("Authorization", headers["Authorization"].as_str().unwrap())
                .json(&effect["body"])
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let payload: Value = response.json().await.map_err(|e| e.to_string())?;
            Ok(json!({"type":"http_result","id":effect["id"],"status":status,"payload":payload}))
        })
    }
}
fn session(url: &str, extra: &[(&str, &str)]) -> (LocalSession, Vec<u8>) {
    let manifest:GuestManifest=serde_json::from_value(json!({"id":"independent","name":"Independent","api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":["http","websocket","channel","delivery","chat","storage","secret_commit"],"services":[url::Url::parse(url).unwrap().origin().ascii_serialization()],"secret_fields":{"access_token":"/access_token"},"subscriptions":["chat.send"],"supported_actions":["chat.send"]})).unwrap();
    let module = include_bytes!("../../../acheflow-server/tests/fixtures/domain.wasm").to_vec();
    let mut config = BTreeMap::from([
        ("server_authorization_fixture".into(), "true".into()),
        ("direct_action_url".into(), url.into()),
    ]);
    for (key, value) in extra {
        config.insert((*key).into(), (*value).into());
    }
    let mut source = LocalSession::new(manifest, &module, config).unwrap();
    source.bind_workspace("workspace").unwrap();
    source
        .set_secret("access_token", "device-only-token".into())
        .unwrap();
    source
        .step(
            json!({"type":"local_server_authorize"}),
            &json!({"browser":false}),
            1000,
        )
        .unwrap();
    (source, module)
}
async fn platform() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let writes = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let writes = writes.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..read]);
                    if data.windows(4).any(|v| v == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&data).to_ascii_lowercase();
                assert!(request.starts_with("post /send"));
                assert!(request.contains("authorization: bearer device-only-token"));
                writes.fetch_add(1, Ordering::SeqCst);
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"id\":\"message\"}").await.unwrap();
            });
        }
    });
    (format!("https://{address}/send"), count, task)
}
#[tokio::test]
async fn real_guest_and_loopback_send_once_with_device_credentials_and_no_collector() {
    let (url, count, task) = platform().await;
    let (source, module) = session(&url, &[]);
    let mut action = source.direct_action(&module, "channel").unwrap();
    let mut http = LoopbackHttp;
    action.prepare(&mut http).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        action
            .send("request", "hello", &mut http, || async { Ok(()) })
            .await
            .unwrap(),
        ExecutionOutcome::Sent
    );
    let _ = action
        .send("request", "hello", &mut http, || async { Ok(()) })
        .await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn revoked_permission_extra_writes_and_unsafe_preparation_never_retry_or_collect() {
    let (url, count, task) = platform().await;
    let (source, module) = session(&url, &[]);
    let mut action = source.direct_action(&module, "channel").unwrap();
    action.prepare(&mut LoopbackHttp).await.unwrap();
    assert!(
        action
            .send("denied", "hello", &mut LoopbackHttp, || async {
                Err("revoked".into())
            })
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    for flag in ["action_prepare_write", "action_prepare_stream"] {
        let (source, module) = session(&url, &[(flag, "true")]);
        let mut action = source.direct_action(&module, "channel").unwrap();
        assert!(action.prepare(&mut LoopbackHttp).await.is_err());
    }
    let (source, module) = session(&url, &[("direct_action_twice", "true")]);
    let mut action = source.direct_action(&module, "channel").unwrap();
    action.prepare(&mut LoopbackHttp).await.unwrap();
    assert_eq!(
        action
            .send("twice", "hello", &mut LoopbackHttp, || async { Ok(()) })
            .await
            .unwrap(),
        ExecutionOutcome::Unknown
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn platform_identity_changes_cannot_replace_the_selected_device_account() {
    let (mut source, module) = session("https://platform.example/send", &[]);
    source
        .config
        .insert("authorized_account".into(), "another-user".into());
    let mut action = source.direct_action(&module, "channel").unwrap();
    assert!(action.prepare(&mut LoopbackHttp).await.is_err());
}

struct ProviderHttp {
    provider: &'static str,
    writes: usize,
    reads: usize,
}
impl ActionHttp for ProviderHttp {
    fn request<'a>(
        &'a mut self,
        effect: &'a Value,
        _: &'a GuestManifest,
        secrets: &'a mut Secrets,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + 'a>> {
        Box::pin(async move {
            assert_eq!(
                secrets.get("access_token").map(String::as_str),
                Some("device-token")
            );
            let write = effect["method"].as_str().unwrap_or("GET") == "POST";
            let id = effect["id"].as_str().unwrap();
            let payload = if write {
                self.writes += 1;
                if self.provider == "twitch" {
                    assert_eq!(effect["body"]["broadcaster_id"], "target-channel");
                    assert_eq!(effect["body"]["sender_id"], "sender");
                    json!({"data":[{"is_sent":true,"message_id":"message"}]})
                } else {
                    json!({"id":"message"})
                }
            } else {
                self.reads += 1;
                match (self.provider, id) {
                    ("youtube", "auth:validate") => {
                        json!({"aud":"application","sub":"sender","scope":"https://www.googleapis.com/auth/youtube.force-ssl","expires_in":3600})
                    }
                    ("youtube", "video") => {
                        json!({"items":[{"id":"target-channel","snippet":{"title":"Target"},"liveStreamingDetails":{"activeLiveChatId":"chat"}}]})
                    }
                    ("vklive", "auth:validate" | "send-permission") => {
                        json!({"data":{"user":{"id":123,"nick":"Sender"}}})
                    }
                    ("vklive", "channel") => {
                        json!({"data":{"channel":{"url":"target-channel","nick":"Target","status":"online"},"stream":{"id":"stream","counters":{"viewers":1}}}})
                    }
                    ("twitch", "auth:validate" | "validate") => {
                        json!({"client_id":"application","user_id":"sender","login":"Sender","scopes":["user:write:chat"],"expires_in":3600})
                    }
                    _ => panic!("unexpected read/subscription: {} {}", self.provider, id),
                }
            };
            Ok(json!({"type":"http_result","id":effect["id"],"status":200,"payload":payload}))
        })
    }
}
#[tokio::test]
async fn actual_provider_guests_resolve_shared_targets_and_send_as_the_local_account_without_reading_chat()
 {
    let catalog = crate::catalog::ClientCatalog::bundled();
    for provider in ["youtube", "vklive", "twitch"] {
        let installed = &catalog.installed[provider];
        let mut source = LocalSession::new(
            installed.manifest.clone(),
            &installed.module,
            BTreeMap::from([("client_id".into(), "application".into())]),
        )
        .unwrap();
        source.bind_workspace("workspace").unwrap();
        source
            .set_secret("access_token", "device-token".into())
            .unwrap();
        let environment = json!({"browser":false});
        let mut http = ProviderHttp {
            provider,
            writes: 0,
            reads: 0,
        };
        let mut effects = source
            .step(json!({"type":"local_server_authorize"}), &environment, 1000)
            .unwrap();
        for _ in 0..8 {
            let Some(request) = effects.iter().find(|e| e["type"] == "http").cloned() else {
                break;
            };
            let (manifest, secrets) = source.http_context();
            let mut response = http.request(&request, manifest, secrets).await.unwrap();
            response["type"] = json!("local_http");
            effects = source.step(response, &environment, 1001).unwrap();
        }
        assert_ne!(source.account_id(), "local-account");
        let mut action = source
            .direct_action(&installed.module, "target-channel")
            .unwrap();
        action
            .prepare(&mut http)
            .await
            .unwrap_or_else(|e| panic!("{provider}: {e}"));
        assert_eq!(http.writes, 0);
        assert_eq!(
            action
                .send("request", "hello", &mut http, || async { Ok(()) })
                .await
                .unwrap(),
            ExecutionOutcome::Sent,
            "{provider}"
        );
        assert_eq!(http.writes, 1);
    }
}
