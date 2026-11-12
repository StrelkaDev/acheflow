use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

#[test]
fn opening_waits_for_transport_confirmation_before_reading() {
    let mut guest = YouTube {
        authorized: true,
        chat: "chat".into(),
        video: "video".into(),
        ..Default::default()
    };
    let effects = guest.handle(input("tick", 1000));
    assert!(effects.iter().any(|e| e["protocol"] == "grpc"));
    assert!(
        effects.iter().all(|e| e["read"] != true),
        "reading before open would bypass handling connection failures"
    );
    let opened = guest.handle(json!({"type":"http_stream_open","id":guest.stream.id}));
    assert!(opened.iter().any(|e| e["read"] == true));
}

fn input(kind: &str, now: u64) -> Value {
    json!({"type":kind,"config":{"video_id":"video"},"secrets":["access_token"],"now_ms":now})
}

#[test]
fn unavailable_grpc_falls_back_to_rest_and_obeys_the_server_poll_interval() {
    let mut guest = setup();
    guest.handle(
        json!({"type":"http_stream_closed","id":guest.stream.id,"http_status":404,"now_ms":2000}),
    );
    let effects = guest.handle(input("tick", 2001));
    let poll = effects
        .iter()
        .find(|e| {
            e["method"] == "GET"
                && e["url"] == "https://www.googleapis.com/youtube/v3/liveChat/messages"
        })
        .expect("unavailable gRPC must not disable receiving chat");
    assert_eq!(poll["query"]["liveChatId"], "chat");
    let effects = guest.handle(json!({"type":"http_result","id":poll["id"],"status":200,"now_ms":2002,"payload":{"pollingIntervalMillis":12000,"nextPageToken":"next-rest","items":[{"id":"remote-message","snippet":{"displayMessage":"From YouTube"},"authorDetails":{"channelId":"user","displayName":"User"}}]}}));
    assert!(
        effects
            .iter()
            .any(|e| e["type"] == "chat" && e["message"]["text"] == "From YouTube")
    );
    assert!(
        guest
            .handle(input("tick", 14001))
            .iter()
            .all(|e| e["url"] != "https://www.googleapis.com/youtube/v3/liveChat/messages")
    );
    let effects = guest.handle(input("tick", 14002));
    assert!(
        effects
            .iter()
            .any(|e| e["method"] == "GET" && e["query"]["pageToken"] == "next-rest")
    );
    assert!(effects.iter().all(|e| e["protocol"] != "grpc"));
}

#[test]
fn offline_discovery_slows_down_and_manual_refresh_is_immediate() {
    let mut guest = YouTube::default();
    guest.handle(auto_input("start", 1000));
    guest.handle(broadcast_response(vec![], 1001));
    assert!(guest.next_video >= 61001);
    assert!(
        guest
            .handle(auto_input("tick", 31001))
            .iter()
            .all(|e| e["id"] != "video")
    );
    let next = guest.next_video;
    guest.handle(auto_input("tick", next));
    guest.handle(broadcast_response(vec![], next + 1));
    assert!(guest.next_video >= next + 120001);
    let effects = guest.handle(
        json!({"type":"action","action_id":"refresh","secrets":["access_token"],"now_ms":next+2}),
    );
    assert!(effects.iter().any(|e| e["id"] == "video"));
}

#[test]
fn rest_recovery_discards_old_broadcast_responses_and_honors_rate_limit_backoff() {
    let mut guest = setup();
    guest.handle(
        json!({"type":"http_stream_closed","id":guest.stream.id,"http_status":404,"now_ms":2000}),
    );
    let effects = guest.handle(input("tick", 2001));
    let id = effects
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .is_some_and(|s| s.starts_with("chat-poll:"))
        })
        .unwrap()["id"]
        .clone();
    guest.handle(
        json!({"type":"http_result","id":id,"status":429,"retry_after":"120","now_ms":2002}),
    );
    assert!(
        guest
            .handle(input("tick", 122001))
            .iter()
            .all(|e| e["url"] != "https://www.googleapis.com/youtube/v3/liveChat/messages")
    );
    let effects = guest.handle(input("tick", 122002));
    let id = effects
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .is_some_and(|s| s.starts_with("chat-poll:"))
        })
        .unwrap()["id"]
        .clone();
    guest.handle(json!({"type":"http_result","id":"video","status":200,"now_ms":122003,"payload":{"items":[{"id":"other-video","liveStreamingDetails":{"activeLiveChatId":"other-chat"}}]}}));
    let effects = guest.handle(json!({"type":"http_result","id":id,"status":401,"now_ms":122004}));
    assert!(
        guest.authorized,
        "late requests cannot expire a new broadcast session"
    );
    assert!(effects.iter().all(|e| e["state"] != "failed"));
}

#[test]
fn broadcast_refresh_does_not_hide_a_failed_chat_stream() {
    let mut guest = setup();
    guest.handle(json!({"type":"http_stream_closed","id":guest.stream.id,"grpc_status":7,"http_status":200,"now_ms":2000}));
    let effects = guest.handle(json!({"type":"http_result","id":"video","status":200,"now_ms":31000,"payload":{"items":[{"id":"video","snippet":{"title":"Live"},"liveStreamingDetails":{"activeLiveChatId":"chat"}}]}}));
    assert!(guest.stream.blocked);
    assert!(
        effects
            .iter()
            .all(|effect| effect["type"] != "status" || effect["state"] != "connected")
    );
}

#[test]
fn stream_transport_errors_are_visible_without_echoing_untrusted_details() {
    for (transport, expected) in [
        ("tls", "TLS negotiation"),
        ("private-token-marker", "stream closed"),
    ] {
        let mut guest = setup();
        let effects = guest.handle(json!({"type":"http_stream_closed","id":guest.stream.id,"grpc_status":14,"http_status":0,"transport_error":transport,"now_ms":2000}));
        let error = effects
            .iter()
            .find(|e| e["state"] == "reconnecting")
            .unwrap()["error"]
            .as_str()
            .unwrap();
        assert!(error.contains("HTTP 0, gRPC 14"));
        assert!(error.contains(expected));
        assert!(!error.contains("private-token-marker"));
    }
}
fn setup() -> YouTube {
    let mut guest = YouTube::default();
    assert!(
        guest
            .handle(input("start", 1000))
            .iter()
            .any(|e| e["url"] == "https://www.googleapis.com/youtube/v3/videos")
    );
    let effects=guest.handle(json!({"type":"http_result","id":"video","status":200,"now_ms":1000,"payload":{"items":[{"id":"video","snippet":{"title":"Live"},"liveStreamingDetails":{"activeLiveChatId":"chat","concurrentViewers":"42"}}]}}));
    assert!(
        effects
            .iter()
            .any(|e| e["type"] == "channel" && e["channel"]["id"] == "video")
    );
    assert!(
        effects
            .iter()
            .any(|e| e["type"] == "audience" && e["sample"]["concurrent"] == 42)
    );
    guest.handle(input("tick", 1000));
    guest.handle(json!({"type":"http_stream_open","id":guest.stream.id,"now_ms":1000}));
    guest
}
fn batch(count: usize, token: &str) -> Vec<u8> {
    let mut batch = vec![];
    for n in 0..count {
        let mut snippet = vec![];
        proto::put(16, format!("Message {n}").as_bytes(), &mut snippet);
        let mut author = vec![0x38, 1];
        proto::put(10101, b"user", &mut author);
        proto::put(103, b"User", &mut author);
        let mut message = vec![];
        proto::put(101, format!("m{n}").as_bytes(), &mut message);
        proto::put(2, &snippet, &mut message);
        proto::put(3, &author, &mut message);
        proto::put(1007, &message, &mut batch);
    }
    proto::put(100602, token.as_bytes(), &mut batch);
    batch
}
fn data(guest: &YouTube, bytes: &[u8], end: bool) -> Value {
    json!({"type":"http_stream_data","id":guest.stream.id,"data":STANDARD.encode(bytes),"end_message":end,"now_ms":1000})
}
#[test]
fn discovers_chat_and_streams_without_periodic_chat_list_requests() {
    let mut guest = setup();
    assert!(guest.stream.active);
    let open = guest.stream.open("chat");
    assert_eq!(
        open["url"],
        "https://youtube.googleapis.com/youtube.api.v3.V3DataLiveChatMessageService/StreamList"
    );
    let request = STANDARD
        .decode(open["body_base64"].as_str().unwrap())
        .unwrap();
    assert!(request.windows(4).any(|w| w == b"chat"));
    assert_eq!(
        open["headers"]["authorization"],
        json!({"secret":"access_token","prefix":"Bearer "})
    );
    for now in [2000, 10_000, 90_000] {
        assert!(
            guest
                .handle(input("tick", now))
                .iter()
                .all(|e| e["type"] != "http_stream" || e["read"] == true)
        );
    }
}
#[test]
fn checkpoint_commits_only_after_draining_and_resumes_with_deduplication() {
    let mut guest = setup();
    let bytes = batch(25, "next+/=");
    let first = guest.handle(data(&guest, &bytes, true));
    assert_eq!(first.iter().filter(|e| e["type"] == "chat").count(), 10);
    assert!(guest.stream.page.is_empty());
    assert!(!first.iter().any(|e| e["read"] == true));
    guest.handle(input("tick", 1001));
    let last = guest.handle(input("tick", 1002));
    assert_eq!(last.iter().filter(|e| e["type"] == "chat").count(), 5);
    assert_eq!(guest.stream.page, "next+/=");
    assert!(last.iter().any(|e| e["read"] == true));
    guest.handle(
        json!({"type":"http_stream_closed","id":guest.stream.id,"grpc_status":14,"now_ms":2000}),
    );
    assert!(
        !guest
            .handle(input("tick", 2999))
            .iter()
            .any(|e| e["protocol"] == "grpc")
    );
    let effects = guest.handle(input("tick", 3000));
    let open = effects.iter().find(|e| e["protocol"] == "grpc").unwrap();
    let wire = STANDARD
        .decode(open["body_base64"].as_str().unwrap())
        .unwrap();
    assert!(wire.windows(7).any(|w| w == b"next+/="));
    assert!(
        guest
            .handle(data(&guest, &bytes, true))
            .iter()
            .all(|e| e["type"] != "chat")
    );
}
#[test]
fn fatal_stream_errors_do_not_retry_until_refresh_and_old_instances_are_ignored() {
    let mut guest = setup();
    let old = guest.stream.id.clone();
    guest.handle(json!({"type":"http_stream_closed","id":old,"grpc_status":9,"now_ms":1000}));
    assert!(guest.stream.blocked);
    assert!(
        guest
            .handle(input("tick", 100000))
            .iter()
            .all(|e| e["protocol"] != "grpc")
    );
    guest.handle(json!({"type":"action","action_id":"refresh","secrets":["access_token"]}));
    guest.handle(input("tick", 100001));
    assert_ne!(old, guest.stream.id);
    assert!(guest.handle(json!({"type":"http_stream_data","id":old,"data":STANDARD.encode(batch(1,"stale")),"end_message":true})).iter().all(|e|e["type"]!="chat"));
    let stop = guest.handle(input("stop", 100002));
    assert!(stop.iter().any(|e| e["close"] == true));
    assert!(
        guest
            .handle(input("tick", 200000))
            .iter()
            .all(|e| e["protocol"] != "grpc")
    );
}
#[test]
fn malformed_and_oversized_batches_do_not_advance_the_checkpoint() {
    for bytes in [
        vec![0],
        vec![0xff; 11],
        vec![0x0a, 3, 1],
        batch(2001, "unsafe"),
    ] {
        let mut guest = setup();
        let mut effects = vec![];
        for (n, chunk) in bytes.chunks(8192).enumerate() {
            effects.extend(guest.handle(data(&guest, chunk, (n + 1) * 8192 >= bytes.len())));
        }
        assert!(effects.iter().any(|e| e["state"] == "failed"));
        assert!(guest.stream.page.is_empty());
    }
}

#[test]
fn chat_ended_event_closes_the_stream_after_the_batch() {
    let mut guest = setup();
    let mut message = vec![];
    proto::put(101, b"ended", &mut message);
    proto::put(2, &[8, 4], &mut message);
    let mut bytes = vec![];
    proto::put(1007, &message, &mut bytes);
    proto::put(100602, b"final", &mut bytes);
    let events = guest.handle(data(&guest, &bytes, true));
    assert!(events.iter().any(|e| e["close"] == true));
    assert!(!guest.stream.active);
    assert!(
        guest
            .handle(input("tick", 2000))
            .iter()
            .all(|e| e["protocol"] != "grpc")
    );
}

#[test]
fn repeated_connection_failures_back_off_to_a_bounded_delay() {
    let mut guest = setup();
    let mut now = 1000;
    for attempt in 0..8 {
        guest.handle(
            json!({"type":"http_stream_closed","id":guest.stream.id,"grpc_status":8,"now_ms":now}),
        );
        let delay = (1000u64 << attempt.min(6)).min(60_000);
        assert_eq!(guest.stream.next, now + delay);
        assert!(
            guest
                .handle(input("tick", now + delay - 1))
                .iter()
                .all(|e| e["protocol"] != "grpc")
        );
        now += delay;
        assert!(
            guest
                .handle(input("tick", now))
                .iter()
                .any(|e| e["protocol"] == "grpc")
        );
    }
}

#[test]
fn disabling_chat_discards_the_pending_batch_and_closes_its_stream() {
    let mut guest = setup();
    let bytes = batch(25, "unfinished");
    guest.handle(data(&guest, &bytes, true));
    assert!(!guest.stream.queue.is_empty());
    let mut disabled = input("tick", 1001);
    disabled["config"]["read_chat"] = json!("false");
    let effects = guest.handle(disabled);
    assert!(effects.iter().any(|effect| effect["close"] == true));
    assert!(effects.iter().all(|effect| effect["type"] != "chat"));
    assert!(guest.stream.queue.is_empty());
    assert!(guest.stream.page.is_empty());
}

fn auto_input(kind: &str, now: u64) -> Value {
    json!({"type":kind,"config":{},"secrets":["access_token"],"now_ms":now})
}

fn active_broadcast(id: &str, chat: &str) -> Value {
    json!({"id":id,"snippet":{"title":format!("Live {id}"),"liveChatId":chat},"status":{"lifeCycleStatus":"live"}})
}

fn broadcast_response(items: Vec<Value>, now: u64) -> Value {
    json!({"type":"http_result","id":"video","status":200,"now_ms":now,"config":{},"payload":{"items":items}})
}

#[test]
fn automatic_discovery_keeps_finding_current_broadcasts_after_the_first_selection() {
    let mut guest = YouTube::default();
    let start = guest.handle(auto_input("start", 1000));
    let lookup = start.iter().find(|e| e["type"] == "http").unwrap();
    assert_eq!(
        lookup["url"],
        "https://www.googleapis.com/youtube/v3/liveBroadcasts"
    );
    assert_eq!(lookup["query"]["broadcastStatus"], "active");
    let selected = guest.handle(broadcast_response(
        vec![active_broadcast("video-a", "chat-a")],
        1001,
    ));
    assert!(
        selected
            .iter()
            .any(|e| e["type"] == "channel" && e["channel"]["id"] == "video-a")
    );
    let next = guest.handle(auto_input("tick", guest.next_video));
    assert!(
        next.iter()
            .any(|e| e["url"] == "https://www.googleapis.com/youtube/v3/liveBroadcasts"),
        "automatic mode must keep looking for new live broadcasts instead of pinning the first video ID"
    );
}

#[test]
fn multiple_active_broadcasts_offer_a_selector_and_switch_to_the_requested_chat() {
    let mut guest = YouTube::default();
    guest.handle(auto_input("start", 1000));
    let effects = guest.handle(broadcast_response(
        vec![
            active_broadcast("video-a", "chat-a"),
            active_broadcast("video-b", "chat-b"),
        ],
        1001,
    ));
    let selector = effects
        .iter()
        .filter(|e| e["type"] == "panel")
        .filter_map(|e| e["panel"]["widgets"].as_array())
        .flatten()
        .find(|w| w["type"] == "select" && w["id"] == "select_broadcast")
        .expect("multiple live broadcasts must be selectable in the guest panel");
    let options = selector["options"].as_array().unwrap();
    for id in ["video-a", "video-b"] {
        assert!(
            options
                .iter()
                .any(|option| option.as_str().is_some_and(|value| value.contains(id))),
            "the selector must distinguish each current video ID"
        );
    }
    let switched = guest.handle(json!({"type":"action","action_id":"select_broadcast","value":"video-b","config":{},"secrets":["access_token"],"now_ms":1002}));
    assert_eq!(guest.video, "video-b");
    assert_eq!(guest.chat, "chat-b");
    assert!(
        switched
            .iter()
            .any(|e| e["type"] == "channel" && e["channel"]["id"] == "video-b")
    );
    let opened = guest.handle(auto_input("tick", 1003));
    let open = opened
        .iter()
        .find(|e| e["protocol"] == "grpc")
        .expect("the selected chat must begin receiving messages");
    let bytes = STANDARD
        .decode(open["body_base64"].as_str().unwrap())
        .unwrap();
    assert!(bytes.windows(6).any(|part| part == b"chat-b"));
}

#[test]
fn broadcast_switch_discards_old_batches_and_late_stream_events() {
    let mut guest = YouTube::default();
    guest.handle(auto_input("start", 1000));
    guest.handle(broadcast_response(
        vec![active_broadcast("video-a", "chat-a")],
        1001,
    ));
    guest.handle(auto_input("tick", 1002));
    let old = guest.stream.id.clone();
    guest.handle(json!({"type":"http_stream_open","id":old,"config":{},"now_ms":1002}));
    let first = guest.handle(data(&guest, &batch(25, "old-page"), true));
    assert_eq!(first.iter().filter(|e| e["type"] == "chat").count(), 10);
    assert!(!guest.stream.queue.is_empty());
    guest.handle(broadcast_response(
        vec![
            active_broadcast("video-a", "chat-a"),
            active_broadcast("video-b", "chat-b"),
        ],
        1003,
    ));
    let switched = guest.handle(json!({"type":"action","action_id":"select_broadcast","value":"video-b","config":{},"secrets":["access_token"],"now_ms":1004}));
    assert_eq!(guest.video, "video-b");
    assert!(
        switched.iter().all(|e| e["type"] != "chat"),
        "queued messages from the previous broadcast must not be relabelled with the new channel"
    );
    assert!(
        switched
            .iter()
            .any(|e| e["close"] == true && e["id"] == old)
    );
    assert!(guest.stream.queue.is_empty());
    assert!(guest.messages.is_empty());
    assert!(guest.stream.page.is_empty());
    guest.handle(auto_input("tick", 1005));
    let late = guest.handle(json!({"type":"http_stream_data","id":old,"data":STANDARD.encode(batch(1,"late-page")),"end_message":true,"config":{},"now_ms":1006}));
    assert!(late.iter().all(|e| e["type"] != "chat"));
}

#[test]
fn automatic_selection_follows_the_remaining_live_broadcast_when_the_old_one_disappears() {
    let mut guest = YouTube::default();
    guest.handle(auto_input("start", 1000));
    guest.handle(broadcast_response(
        vec![active_broadcast("video-a", "chat-a")],
        1001,
    ));
    guest.handle(auto_input("tick", 1002));
    let old = guest.stream.id.clone();
    guest.handle(json!({"type":"http_stream_open","id":old,"config":{},"now_ms":1002}));
    let selected = guest.handle(broadcast_response(
        vec![active_broadcast("video-b", "chat-b")],
        32000,
    ));
    assert_eq!(guest.video, "video-b");
    assert_eq!(guest.chat, "chat-b");
    assert!(
        selected
            .iter()
            .any(|e| e["close"] == true && e["id"] == old)
    );
    assert!(
        selected
            .iter()
            .any(|e| e["type"] == "channel" && e["channel"]["id"] == "video-b")
    );
    let missing = guest.handle(broadcast_response(vec![], 64000));
    assert!(guest.chat.is_empty());
    assert!(missing.iter().any(|e| e["state"] == "connecting"));
    assert!(
        guest
            .handle(auto_input("tick", guest.next_video))
            .iter()
            .any(|e| e["url"] == "https://www.googleapis.com/youtube/v3/liveBroadcasts")
    );
}

#[test]
fn explicit_video_override_remains_pinned_to_the_requested_video() {
    let mut guest = setup();
    let next = guest.handle(input("tick", guest.next_video));
    let lookup = next
        .iter()
        .find(|e| e["id"] == "video" && e["type"] == "http")
        .unwrap();
    assert_eq!(
        lookup["url"],
        "https://www.googleapis.com/youtube/v3/videos"
    );
    assert_eq!(lookup["query"]["id"], "video");
}
