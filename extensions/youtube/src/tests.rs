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
