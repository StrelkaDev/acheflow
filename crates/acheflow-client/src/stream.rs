//! Provider-neutral, demand-driven gRPC framing. Protobuf belongs to the guest.
use crate::transport::{self, Secrets};
use acheflow_extension_api::integration::GuestManifest;
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Buf, Bytes};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::time::Duration;

pub(crate) const MAX_MESSAGE: usize = 512 * 1024;
const SEGMENT: usize = 8192;
pub(crate) struct Stream {
    id: String,
    body: reqwest::Body,
    chunk: Bytes,
    header: Vec<u8>,
    remaining: Option<usize>,
    ready: bool,
    _driver: Option<Driver>,
}
struct Driver(tokio::task::JoinHandle<()>);
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn connect(
    url: &url::Url,
) -> Result<
    (
        hyper::client::conn::http2::SendRequest<http_body_util::Full<Bytes>>,
        Driver,
    ),
    String,
> {
    let host = url.host_str().ok_or("missing stream host")?;
    let tcp = tokio::net::TcpStream::connect((
        host,
        url.port_or_known_default().ok_or("invalid stream port")?,
    ))
    .await
    .map_err(|_| "stream connection failed")?;
    let tls = native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .request_alpns(&["h2"])
        .build()
        .map_err(|_| "stream TLS initialization failed")?;
    let tls = tokio_native_tls::TlsConnector::from(tls)
        .connect(host, tcp)
        .await
        .map_err(|_| "stream TLS failed")?;
    if tls
        .get_ref()
        .negotiated_alpn()
        .map_err(|_| "stream ALPN failed")?
        .as_deref()
        != Some(b"h2")
    {
        return Err("stream requires HTTP/2".into());
    }
    let mut protocol =
        hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new());
    let (sender, connection) = protocol
        .initial_stream_window_size(65_535)
        .initial_connection_window_size(65_535)
        .max_frame_size(16_384)
        .max_header_list_size(16_384)
        .handshake(hyper_util::rt::TokioIo::new(tls))
        .await
        .map_err(|_| "stream HTTP/2 initialization failed")?;
    let driver = Driver(tokio::spawn(async move {
        let _ = connection.await;
    }));
    Ok((sender, driver))
}
struct Prepared {
    id: String,
    url: url::Url,
    request: hyper::Request<http_body_util::Full<Bytes>>,
}
fn prepare(
    effect: &Value,
    manifest: &GuestManifest,
    secrets: &Secrets,
) -> Result<Prepared, String> {
    let id = effect["id"]
        .as_str()
        .filter(|id| acheflow_extension_api::integration::valid_id(id, 128))
        .ok_or("invalid stream ID")?;
    if effect["protocol"] != "grpc" || effect.get("capture").is_some() {
        return Err("unsupported streaming protocol or credential capture".into());
    }
    let url = transport::allowed_url(
        effect["url"].as_str().ok_or("missing stream URL")?,
        manifest,
        false,
    )?;
    if url.query().is_some() {
        return Err("stream parameters belong to the request message".into());
    }
    let encoded = effect["body_base64"]
        .as_str()
        .filter(|s| s.len() <= 90_000)
        .ok_or("invalid stream request")?;
    let data = STANDARD
        .decode(encoded)
        .map_err(|_| "invalid stream request encoding")?;
    if data.len() > 65536 {
        return Err("stream request exceeds 64 KiB".into());
    }
    let mut body = vec![0];
    body.extend_from_slice(&(data.len() as u32).to_be_bytes());
    body.extend_from_slice(&data);
    let mut request = hyper::Request::builder()
        .method("POST")
        .version(hyper::Version::HTTP_2)
        .uri(url.as_str());
    let headers = transport::resolve(&effect["headers"], secrets)?;
    if let Some(headers) = headers.as_object() {
        if headers.len() > 32 {
            return Err("too many stream headers".into());
        }
        for (name, value) in headers {
            transport::validate_header(name)?;
            if name.to_ascii_lowercase().starts_with("grpc-")
                || name.eq_ignore_ascii_case("te")
                || name.eq_ignore_ascii_case("content-type")
            {
                return Err("reserved stream header".into());
            }
            let mut header = hyper::header::HeaderValue::from_str(
                value.as_str().ok_or("invalid stream header")?,
            )
            .map_err(|_| "invalid stream header")?;
            header.set_sensitive(true);
            request = request.header(name, header);
        }
    }
    let request = request
        .header("content-type", "application/grpc+proto")
        .header("te", "trailers")
        .header("grpc-accept-encoding", "identity")
        .body(http_body_util::Full::new(Bytes::from(body)))
        .map_err(|_| "invalid stream request headers")?;
    Ok(Prepared {
        id: id.to_owned(),
        url,
        request,
    })
}

fn response_status<B>(id: &str, response: &hyper::Response<B>) -> Result<Option<Value>, String> {
    let status = response.status().as_u16();
    if status != 200 {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|s| s.clamp(1, 3600) * 1000);
        return Ok(Some(
            json!({"type":"http_stream_closed","id":id,"http_status":status,"grpc_status":null,"retry_after_ms":retry}),
        ));
    }
    if response.version() != reqwest::Version::HTTP_2
        || !matches!(
            response
                .headers()
                .get("content-type")
                .and_then(|s| s.to_str().ok()),
            Some("application/grpc" | "application/grpc+proto")
        )
    {
        return Err("invalid streaming response protocol".into());
    }
    if let Some(code) = response.headers().get("grpc-status") {
        let code = code
            .to_str()
            .ok()
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|c| *c <= 16)
            .unwrap_or(13);
        return Ok(Some(closed(id, code)));
    }
    Ok(None)
}

impl Stream {
    pub(crate) async fn open(
        effect: &Value,
        manifest: &GuestManifest,
        secrets: &Secrets,
    ) -> Result<(Option<Self>, Value), String> {
        let Prepared { id, url, request } = prepare(effect, manifest, secrets)?;
        let (response, driver) = tokio::time::timeout(Duration::from_secs(10), async {
            let (mut sender, driver) = connect(&url).await?;
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| "stream request interrupted")?;
            Ok::<_, String>((response, driver))
        })
        .await
        .map_err(|_| "stream connection timed out")??;
        if let Some(event) = response_status(&id, &response)? {
            return Ok((None, event));
        }
        let mut stream = Self::new(id.clone(), reqwest::Body::wrap(response.into_body()));
        stream._driver = Some(driver);
        Ok((Some(stream), json!({"type":"http_stream_open","id":id})))
    }
    pub(crate) fn new(id: String, body: reqwest::Body) -> Self {
        Self {
            id,
            body,
            chunk: Bytes::new(),
            header: vec![],
            remaining: None,
            ready: false,
            _driver: None,
        }
    }
    pub(crate) fn resume(&mut self, id: &Value) -> Result<(), String> {
        if id.as_str() != Some(&self.id) {
            return Err("stream ID changed".into());
        }
        self.ready = true;
        Ok(())
    }
    fn segment(&mut self) -> Result<Option<Value>, String> {
        if self.remaining.is_none() && !self.chunk.is_empty() {
            let count = (5 - self.header.len()).min(self.chunk.len());
            self.header.extend_from_slice(&self.chunk[..count]);
            self.chunk.advance(count);
            if self.header.len() == 5 {
                let length = u32::from_be_bytes(
                    self.header[1..5]
                        .try_into()
                        .map_err(|_| "invalid stream header")?,
                ) as usize;
                if self.header[0] != 0 || length > MAX_MESSAGE {
                    return Err("unsupported or oversized streaming message".into());
                }
                self.remaining = Some(length);
            }
        }
        if let Some(remaining) = self.remaining
            && (remaining == 0 || !self.chunk.is_empty())
        {
            let count = remaining.min(self.chunk.len()).min(SEGMENT);
            let data = STANDARD.encode(&self.chunk[..count]);
            self.chunk.advance(count);
            let end = count == remaining;
            self.remaining = if end {
                self.header.clear();
                None
            } else {
                Some(remaining - count)
            };
            self.ready = false;
            return Ok(Some(
                json!({"type":"http_stream_data","id":self.id,"data":data,"end_message":end}),
            ));
        }
        Ok(None)
    }
    async fn read(&mut self) -> Result<Value, String> {
        if !self.ready {
            std::future::pending::<()>().await;
        }
        loop {
            if let Some(event) = self.segment()? {
                return Ok(event);
            }
            let frame = self
                .body
                .frame()
                .await
                .ok_or("stream ended without status")?
                .map_err(|_| "stream body interrupted")?;
            match frame.into_data() {
                Ok(data) => self.chunk = data,
                Err(frame) => {
                    let trailers = frame.into_trailers().map_err(|_| "invalid stream frame")?;
                    if self.remaining.is_some() || !self.header.is_empty() {
                        return Err("truncated streaming message".into());
                    }
                    let code = trailers
                        .get("grpc-status")
                        .and_then(|s| s.to_str().ok())
                        .and_then(|s| s.parse::<u8>().ok())
                        .filter(|c| *c <= 16)
                        .ok_or("missing stream status")?;
                    return Ok(closed(&self.id, code));
                }
            }
        }
    }
}
fn closed(id: &str, status: u8) -> Value {
    json!({"type":"http_stream_closed","id":id,"http_status":200,"grpc_status":status})
}
pub(crate) async fn receive(stream: &mut Option<Stream>) -> Value {
    let Some(active) = stream.as_mut() else {
        return std::future::pending().await;
    };
    let event = active
        .read()
        .await
        .unwrap_or_else(|_| closed(&active.id, 13));
    if event["type"] == "http_stream_closed" {
        *stream = None;
    }
    event
}

#[cfg(test)]
mod tests;
