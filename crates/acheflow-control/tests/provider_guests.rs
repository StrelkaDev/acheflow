//! Provider fixtures exercise the actual independently compiled Rust WASM guests.
use acheflow_control::extensions::{Guest, GuestManifest};
use serde_json::{Value, json};
#[path = "support/youtube_stream.rs"]
mod youtube_stream;

fn load(id: &str) -> Guest {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions")
        .join(id);
    let manifest: GuestManifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    manifest.validate().unwrap();
    Guest::load(
        &std::fs::read(root.join(&manifest.module)).unwrap(),
        manifest.capabilities,
    )
    .unwrap()
}
fn start(guest: &mut Guest, id: &str) -> Vec<Value> {
    guest.call(json!({"type":"start","config":if id=="youtube" {json!({"video_id":"video"})}else{json!({"channel":"streamer"})},"secrets":["access_token"],"now_ms":1000})).unwrap()
}

#[test]
fn youtube_rest_recovery_receives_a_full_page_in_real_wasm_and_rejects_late_results() {
    let mut guest = load("youtube");
    start(&mut guest, "youtube");
    guest.call(json!({"type":"http_result","id":"video","status":200,"now_ms":1000,"payload":{"items":[{"id":"video","liveStreamingDetails":{"activeLiveChatId":"chat"}}]}})).unwrap();
    let effects = guest.call(json!({"type":"tick","now_ms":1001})).unwrap();
    let id = effects.iter().find(|e| e["protocol"] == "grpc").unwrap()["id"].clone();
    guest
        .call(json!({"type":"http_stream_closed","id":id,"http_status":404,"now_ms":1002}))
        .unwrap();
    let effects = guest.call(json!({"type":"tick","now_ms":1003})).unwrap();
    let poll = effects
        .iter()
        .find(|e| {
            e["method"] == "GET"
                && e["url"] == "https://www.googleapis.com/youtube/v3/liveChat/messages"
        })
        .unwrap();
    let result = json!({"type":"http_result","id":poll["id"],"status":200,"now_ms":1004,"payload":{"pollingIntervalMillis":120000,"nextPageToken":"next","items":(0..200).map(|i|json!({"id":format!("rest-{i}"),"snippet":{"displayMessage":"x".repeat(160)},"authorDetails":{"channelId":"user","displayName":"User"}})).collect::<Vec<_>>()}});
    let mut received = guest
        .call(result.clone())
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "chat")
        .count();
    for now in 1005..1024 {
        received += guest
            .call(json!({"type":"tick","now_ms":now}))
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "chat")
            .count();
    }
    assert_eq!(received, 200);
    guest.call(json!({"type":"stop"})).unwrap();
    assert!(
        guest
            .call(result)
            .unwrap()
            .iter()
            .all(|e| e["type"] != "chat")
    );
}
#[test]
fn independently_compiled_providers_use_opaque_credentials_and_declare_only_supported_channels() {
    for id in ["youtube", "vklive"] {
        let mut guest = load(id);
        let effects = start(&mut guest, id);
        let request = effects.iter().find(|e| e["type"] == "http").unwrap();
        assert_eq!(
            request["headers"]["Authorization"],
            json!({"secret":"access_token","prefix":"Bearer "})
        );
        assert!(effects.iter().any(|e| e["type"] == "panel"));
        let response = if id == "youtube" {
            json!({"type":"http_result","id":"video","status":200,"now_ms":1000,"payload":{"items":[{"id":"video","snippet":{"title":"Live"},"liveStreamingDetails":{"activeLiveChatId":"chat","concurrentViewers":"42"}}]}})
        } else {
            json!({"type":"http_result","id":"channel","status":200,"now_ms":1000,"payload":{"data":{"channel":{"url":"streamer","nick":"Streamer","status":"online"},"stream":{"id":"stream","counters":{"viewers":17}}}}})
        };
        let effects = guest.call(response).unwrap();
        let channel: acheflow_extension_api::integration::Channel = serde_json::from_value(
            effects
                .iter()
                .find(|e| e["type"] == "channel" && e["channel"].is_object())
                .unwrap()["channel"]
                .clone(),
        )
        .unwrap();
        assert_eq!(
            channel.capabilities,
            if id == "youtube" {
                vec![
                    "chat.read".to_string(),
                    "chat.send".to_string(),
                    "audience.read".to_string(),
                ]
            } else {
                vec!["chat.read".to_string(), "audience.read".to_string()]
            }
        );
        let sample: acheflow_extension_api::audience::AudienceSample = serde_json::from_value(
            effects.iter().find(|e| e["type"] == "audience").unwrap()["sample"].clone(),
        )
        .unwrap();
        assert_eq!(
            sample.concurrent,
            Some(if id == "youtube" { 42 } else { 17 })
        );
        let effects = guest.call(json!({"type":"http_result","id":if id=="youtube"{"chat"}else{"channel"},"status":401})).unwrap();
        assert!(
            effects
                .iter()
                .any(|e| e["type"] == "status" && e["state"] == "failed")
        );
        assert!(
            guest
                .call(json!({"type":"tick","now_ms":100000}))
                .unwrap()
                .iter()
                .all(|e| e["type"] != "http")
        );
    }
}

#[test]
fn youtube_preserves_every_message_across_bounded_wasm_calls_and_correlates_sends() {
    let mut guest = load("youtube");
    start(&mut guest, "youtube");
    guest.call(json!({"type":"http_result","id":"video","status":200,"payload":{"items":[{"id":"video","liveStreamingDetails":{"activeLiveChatId":"chat"}}]}})).unwrap();
    let open = guest.call(json!({"type":"tick","now_ms":1000})).unwrap();
    let id = open
        .iter()
        .find(|effect| effect["protocol"] == "grpc")
        .unwrap()["id"]
        .clone();
    guest
        .call(json!({"type":"http_stream_open","id":id,"now_ms":1000}))
        .unwrap();
    let bytes = youtube_stream::batch(2000, 160, "resume_checkpoint_+/=");
    let mut received = vec![];
    for (index, chunk) in bytes.chunks(8192).enumerate() {
        let effects = guest
            .call(youtube_stream::event(
                &id,
                chunk,
                (index + 1) * 8192 >= bytes.len(),
            ))
            .unwrap();
        for effect in effects {
            if effect["type"] == "chat" {
                received.push(effect["message"]["message_id"].as_str().unwrap().to_owned());
            }
        }
    }
    for _ in 0..199 {
        let effects = guest.call(json!({"type":"tick","now_ms":1000})).unwrap();
        assert!(effects.len() <= 64);
        for effect in &effects {
            if effect["type"] == "chat" {
                let message: acheflow_extension_api::interaction::ChatMessage =
                    serde_json::from_value(effect["message"].clone()).unwrap();
                received.push(message.message_id);
            }
        }
    }
    assert_eq!(received.len(), 2000);
    assert_eq!(received[1999], "m1999");
    guest
        .call(json!({"type":"http_stream_closed","id":id,"grpc_status":14,"now_ms":1000}))
        .unwrap();
    let reopened = guest.call(json!({"type":"tick","now_ms":2000})).unwrap();
    let open = reopened.iter().find(|e| e["protocol"] == "grpc").unwrap();
    let token = b"resume_checkpoint_+/=";
    assert!(
        youtube_stream::decoded_request(open)
            .windows(token.len())
            .any(|s| s == token)
    );
    let id = open["id"].clone();
    guest
        .call(json!({"type":"http_stream_open","id":id,"now_ms":2000}))
        .unwrap();
    for (index, chunk) in bytes.chunks(8192).enumerate() {
        let effects = guest
            .call(youtube_stream::event(
                &id,
                chunk,
                (index + 1) * 8192 >= bytes.len(),
            ))
            .unwrap();
        assert!(effects.iter().all(|e| e["type"] != "chat"));
    }
    for _ in 0..199 {
        assert!(
            guest
                .call(json!({"type":"tick","now_ms":2000}))
                .unwrap()
                .iter()
                .all(|e| e["type"] != "chat")
        );
    }
    let effects=guest.call(json!({"type":"command","name":"chat.send","channel_id":"video","request_id":"intent","text":"Hello"})).unwrap();
    let request = effects.iter().find(|e| e["type"] == "http").unwrap();
    assert_eq!(
        request["body"]["snippet"]["textMessageDetails"]["messageText"],
        "Hello"
    );
    let effects=guest.call(json!({"type":"http_result","id":"send:intent","status":200,"payload":{"id":"external-message"}})).unwrap();
    assert!(effects.iter().any(|e| e["type"] == "delivery"
        && e["request_id"] == "intent"
        && e["external_id"] == "external-message"
        && e["sent"] == true));
}

#[test]
fn vklive_preserves_history_pages_within_the_fixed_fuel_budget() {
    let mut guest = load("vklive");
    start(&mut guest, "vklive");
    let messages: Vec<_> = (1..=200).map(|n|json!({"id":n,"created_at":n,"author":{"id":7,"nick":"User","is_moderator":false},"parts":[{"text":{"content":format!("Message {n}")}}]})).collect();
    let mut effects=guest.call(json!({"type":"http_result","id":"chat","status":200,"payload":{"data":{"chat_messages":messages}}})).unwrap();
    let mut received = 0;
    for _ in 0..20 {
        received += effects.iter().filter(|e| e["type"] == "chat").count();
        effects = guest.call(json!({"type":"tick","now_ms":1000})).unwrap();
    }
    assert_eq!(received, 200);
}
