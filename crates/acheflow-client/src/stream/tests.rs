use super::*;
use futures_util::{FutureExt, stream};
use hyper::body::Frame;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

fn framed(data: &[u8]) -> Vec<u8> {
    let mut wire = vec![0];
    wire.extend_from_slice(&(data.len() as u32).to_be_bytes());
    wire.extend_from_slice(data);
    wire
}
fn body(frames: Vec<Frame<Bytes>>) -> reqwest::Body {
    reqwest::Body::wrap(http_body_util::StreamBody::new(stream::iter(
        frames.into_iter().map(Ok::<_, std::io::Error>),
    )))
}
fn trailers(code: &str) -> Frame<Bytes> {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("grpc-status", code.parse().unwrap());
    Frame::trailers(headers)
}
#[test]
fn request_uses_baseline_grpc_media_type_for_endpoint_interoperability() {
    let manifest: GuestManifest = serde_json::from_value(json!({
        "id":"fixture", "name":"Fixture", "api_version":2,
        "module":"fixture.wasm", "execution_side":"client",
        "capabilities":["http_stream"], "services":["https://provider.example"]
    }))
    .unwrap();
    let effect = json!({
        "id":"stream", "protocol":"grpc",
        "url":"https://provider.example/service/Read",
        "body_base64":STANDARD.encode(b"request"), "headers":{}
    });
    let prepared = prepare(&effect, &manifest, &Secrets::new()).unwrap();
    assert_eq!(
        prepared.request.headers()["content-type"],
        "application/grpc"
    );
}
#[test]
fn grpc_content_type_accepts_parameters_but_rejects_other_protocols() {
    for content_type in [
        "application/grpc",
        "application/grpc+proto",
        "application/grpc; charset=utf-8",
        "application/grpc+proto; charset=utf-8",
    ] {
        let response = hyper::Response::builder()
            .version(hyper::Version::HTTP_2)
            .header("content-type", content_type)
            .body(())
            .unwrap();
        assert!(
            response_status("stream", &response).unwrap().is_none(),
            "{content_type}"
        );
    }
    for content_type in [
        "application/grpc-web+proto",
        "application/grpc+json",
        "text/html",
    ] {
        let response = hyper::Response::builder()
            .version(hyper::Version::HTTP_2)
            .header("content-type", content_type)
            .body(())
            .unwrap();
        assert!(response_status("stream", &response).is_err());
    }
}

#[test]
fn open_failure_reports_only_fixed_transport_categories() {
    for (error, category) in [
        ("stream connection failed", "network"),
        ("stream TLS failed", "tls"),
        ("stream requires HTTP/2", "http2"),
        ("stream connection timed out", "timeout"),
        ("invalid streaming response protocol", "protocol"),
        ("private-token-marker", "invalid_request"),
    ] {
        let event = open_failure(&json!("stream"), error);
        assert_eq!(event["transport_error"], category);
        assert_eq!(event["grpc_status"], 14);
        assert!(!event.to_string().contains("private-token-marker"));
    }
}
#[test]
fn status_metadata_is_bounded_redacted_and_streaming_is_client_only() {
    let mut response = hyper::Response::builder()
        .version(hyper::Version::HTTP_2)
        .header("content-type", "application/grpc+proto")
        .header("grpc-status", "16")
        .header("grpc-message", "private-token-marker")
        .body(())
        .unwrap();
    let event = response_status("stream", &response).unwrap().unwrap();
    assert_eq!(event["grpc_status"], 16);
    assert!(!event.to_string().contains("private-token-marker"));
    *response.status_mut() = hyper::StatusCode::TOO_MANY_REQUESTS;
    response
        .headers_mut()
        .insert("retry-after", "30".parse().unwrap());
    assert_eq!(
        response_status("stream", &response).unwrap().unwrap()["retry_after_ms"],
        30_000
    );
    let manifest:GuestManifest=serde_json::from_value(json!({"id":"logic","name":"Logic","api_version":2,"module":"logic.wasm","capabilities":["http_stream"],"services":[]})).unwrap();
    assert!(manifest.validate().is_err());
}
#[test]
fn resolved_credentials_are_not_indexed_or_visible_in_request_debug_output() {
    let manifest:GuestManifest=serde_json::from_value(json!({"id":"fixture","name":"Fixture","api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":["http_stream"],"services":["https://provider.example"]})).unwrap();
    let effect = json!({"id":"stream","protocol":"grpc","url":"https://provider.example/service/Read","body_base64":"","headers":{"authorization":{"secret":"access","prefix":"Bearer "}}});
    let prepared = prepare(
        &effect,
        &manifest,
        &Secrets::from([("access".into(), "private-marker".into())]),
    )
    .unwrap();
    assert!(prepared.request.headers()["authorization"].is_sensitive());
    assert!(!format!("{:?}", prepared.request.headers()).contains("private-marker"));
}
#[tokio::test]
async fn dropping_stream_aborts_the_owned_http2_driver() {
    struct Notice(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Notice {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let notice = Notice(Some(sender));
    let task = tokio::spawn(async move {
        let _notice = notice;
        std::future::pending::<()>().await;
    });
    let mut stream = Stream::new("id".into(), reqwest::Body::from(""));
    stream._driver = Some(Driver(task));
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), receiver)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn split_headers_coalesced_frames_and_backpressure_preserve_exact_bytes() {
    let large = vec![42; SEGMENT + 7];
    let mut wire = framed(&large);
    wire.extend(framed(b"second"));
    let mut stream = Some(Stream::new(
        "chat:1".into(),
        body(vec![
            Frame::data(Bytes::copy_from_slice(&wire[..2])),
            Frame::data(Bytes::copy_from_slice(&wire[2..])),
            trailers("0"),
        ]),
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), receive(&mut stream))
            .await
            .is_err()
    );
    let mut collected = vec![];
    for (end, count) in [(false, SEGMENT), (true, 7), (true, 6)] {
        stream.as_mut().unwrap().resume(&json!("chat:1")).unwrap();
        let event = receive(&mut stream).await;
        let bytes = STANDARD.decode(event["data"].as_str().unwrap()).unwrap();
        assert_eq!(bytes.len(), count);
        assert_eq!(event["end_message"], end);
        collected.extend(bytes);
    }
    assert_eq!(collected, [large, b"second".to_vec()].concat());
    stream.as_mut().unwrap().resume(&json!("chat:1")).unwrap();
    assert_eq!(receive(&mut stream).await["grpc_status"], 0);
    assert!(stream.is_none());
}
#[tokio::test]
async fn malformed_compressed_oversized_truncated_or_unterminated_streams_close_safely() {
    for wire in [
        vec![1, 0, 0, 0, 0],
        vec![0, 0, 8, 0, 1],
        vec![0, 0, 0],
        vec![0, 0, 0, 0, 3, b'a'],
    ] {
        let mut stream = Some(Stream::new(
            "id".into(),
            body(vec![Frame::data(Bytes::from(wire)), trailers("0")]),
        ));
        stream.as_mut().unwrap().resume(&json!("id")).unwrap();
        // A partial payload may be delivered, but it cannot be committed as a message.
        let mut event = receive(&mut stream).await;
        if event["type"] == "http_stream_data" {
            stream.as_mut().unwrap().resume(&json!("id")).unwrap();
            event = receive(&mut stream).await;
        }
        assert_eq!(event["grpc_status"], 13);
        assert!(stream.is_none());
    }
    let mut stream = Some(Stream::new("id".into(), body(vec![trailers("8")])));
    stream.as_mut().unwrap().resume(&json!("id")).unwrap();
    assert_eq!(receive(&mut stream).await["grpc_status"], 8);
}
struct Gated {
    frames: VecDeque<Frame<Bytes>>,
    released: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    first: bool,
}
impl Drop for Gated {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}
impl hyper::body::Body for Gated {
    type Data = Bytes;
    type Error = std::io::Error;
    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        if this.first || this.released.load(Ordering::Acquire) {
            this.first = false;
            Poll::Ready(this.frames.pop_front().map(Ok))
        } else {
            Poll::Pending
        }
    }
}
#[tokio::test]
async fn cancelling_a_receive_retains_partial_framing_and_drop_cancels_owned_io() {
    let released = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let wire = framed(b"message");
    let body = reqwest::Body::wrap(Gated {
        frames: VecDeque::from([
            Frame::data(Bytes::copy_from_slice(&wire[..2])),
            Frame::data(Bytes::copy_from_slice(&wire[2..])),
            trailers("0"),
        ]),
        released: released.clone(),
        dropped: dropped.clone(),
        first: true,
    });
    let mut active = Stream::new("id".into(), body);
    active.resume(&json!("id")).unwrap();
    assert!(active.read().now_or_never().is_none());
    assert_eq!(active.header.len(), 2);
    released.store(true, Ordering::Release);
    assert_eq!(
        STANDARD
            .decode(active.read().await.unwrap()["data"].as_str().unwrap())
            .unwrap(),
        b"message"
    );
    assert!(active.resume(&json!("replacement")).is_err());
    drop(active);
    assert!(dropped.load(Ordering::Acquire));
}
#[tokio::test]
async fn forbidden_origins_and_secret_or_transport_header_misuse_never_connect() {
    let manifest:GuestManifest=serde_json::from_value(json!({"id":"fixture","name":"Fixture","api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":["http_stream"],"services":["https://provider.example"]})).unwrap();
    let base = json!({"id":"stream","protocol":"grpc","url":"https://provider.example/service/Read","body_base64":"","headers":{}});
    for changed in [
        json!({"url":"https://denied.example/service/Read"}),
        json!({"headers":{"host":"elsewhere"}}),
        json!({"headers":{"grpc-encoding":"gzip"}}),
        json!({"capture":{"token":"/token"}}),
        json!({"headers":{"authorization":{"secret":"missing"}}}),
    ] {
        let mut effect = base.clone();
        effect
            .as_object_mut()
            .unwrap()
            .extend(changed.as_object().unwrap().clone());
        assert!(
            Stream::open(&effect, &manifest, &Secrets::new())
                .await
                .is_err()
        );
    }
}
