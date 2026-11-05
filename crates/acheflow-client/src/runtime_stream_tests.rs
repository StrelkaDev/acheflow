use super::*;
#[path = "../../acheflow-control/tests/support/youtube_stream.rs"]
mod fixture;

fn runtime(id: &str, channel: &str) -> ProviderRuntime {
    let catalog = crate::catalog::ClientCatalog::bundled();
    let installed = &catalog.installed[id];
    ProviderRuntime::new(
        installed.manifest.clone(),
        &installed.module,
        Identity {
            workspace_id: "workspace".into(),
            client_id: "client".into(),
            generation: 1,
            connection: Connection {
                id: "local".into(),
                provider: id.into(),
                account_id: "account".into(),
                channel_id: channel.into(),
                capabilities: [Capability::ChatRead].into(),
            },
        },
        BTreeMap::from([("video_id".into(), "video".into())]),
    )
    .unwrap()
}

#[tokio::test]
async fn real_guests_receive_only_owned_streams_and_stop_discards_late_messages() {
    let mut youtube = runtime("youtube", "video");
    let mut twitch = runtime("twitch", "b");
    youtube.guest.call(json!({"type":"start","config":{"video_id":"video"},"secrets":["access_token"],"now_ms":1000})).unwrap();
    youtube.guest.call(json!({"type":"http_result","id":"video","status":200,"now_ms":1000,"payload":{"items":[{"id":"video","liveStreamingDetails":{"activeLiveChatId":"chat"}}]}})).unwrap();
    let opened = youtube
        .guest
        .call(json!({"type":"tick","now_ms":1000}))
        .unwrap();
    let open = opened.iter().find(|e| e["protocol"] == "grpc").unwrap();
    let id = open["id"].clone();
    assert!(!fixture::decoded_request(open).is_empty());
    let bytes = fixture::batch(2, 1, "checkpoint");
    let mut wire = vec![0];
    wire.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    wire.extend_from_slice(&bytes);
    youtube.stream = Some(crate::stream::Stream::new(
        id.as_str().unwrap().into(),
        reqwest::Body::from(wire),
    ));
    youtube
        .turn(json!({"type":"http_stream_open","id":id}), 1000)
        .await
        .unwrap();
    let events = youtube.receive(1001).await.unwrap();
    assert_eq!(events.iter().filter(|e| e["type"] == "chat").count(), 2);
    assert!(events.iter().all(|e| e["type"] != "http_stream"));
    assert!(
        events
            .iter()
            .filter(|e| e["type"] == "chat")
            .all(|e| e["message"]["platform"] == "youtube")
    );
    let events=twitch.turn(json!({"type":"websocket","payload":{"metadata":{"message_type":"notification","subscription_type":"channel.chat.message"},"payload":{"event":{"broadcaster_user_id":"b","message_id":"t1","chatter_user_id":"user","chatter_user_name":"User","message":{"text":"Twitch message"},"badges":[]}}}}),1002).await.unwrap();
    assert_eq!(events.iter().filter(|e| e["type"] == "chat").count(), 1);
    assert!(
        events
            .iter()
            .filter(|e| e["type"] == "chat")
            .all(|e| e["message"]["platform"] == "twitch")
    );
    assert!(twitch.stream.is_none());
    youtube.turn(json!({"type":"stop"}), 1003).await.unwrap();
    assert!(youtube.stream.is_none());
    assert!(
        youtube
            .turn(fixture::event(&id, &bytes, true), 1004)
            .await
            .unwrap()
            .iter()
            .all(|e| e["type"] != "chat")
    );
}
