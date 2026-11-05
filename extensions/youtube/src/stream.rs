use super::{YouTube, proto, status};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct ChatStream {
    pub active: bool,
    pub id: String,
    pub next: u64,
    pub page: String,
    pub queue: VecDeque<Vec<u8>>,
    pub waiting: bool,
    pub blocked: bool,
    bytes: Vec<u8>,
    token: String,
    offline: bool,
    failures: u8,
    serial: u64,
}
impl ChatStream {
    pub fn can_open(&self, now: u64) -> bool {
        if self.active || self.blocked {
            return false;
        }
        now >= self.next && self.queue.is_empty()
    }
    pub fn close(&mut self) -> Option<Value> {
        let active = self.active;
        self.active = false;
        self.waiting = false;
        self.bytes.clear();
        active.then(|| json!({"type":"http_stream","id":self.id,"close":true}))
    }
    pub fn reset(&mut self) -> Option<Value> {
        let close = self.close();
        self.page.clear();
        self.token.clear();
        self.queue.clear();
        self.failures = 0;
        self.next = 0;
        self.offline = false;
        self.blocked = false;
        close
    }
    pub fn open(&mut self, chat: &str) -> Value {
        self.active = true;
        self.waiting = true;
        self.serial += 1;
        self.id = format!("chat:{}", self.serial);
        json!({"type":"http_stream","id":self.id,"protocol":"grpc","url":"https://youtube.googleapis.com/youtube.api.v3.V3DataLiveChatMessageService/StreamList","headers":{"authorization":{"secret":"access_token","prefix":"Bearer "}},"body_base64":STANDARD.encode(proto::request(chat, &self.page))})
    }
    fn read(&mut self) -> Option<Value> {
        if !self.active || self.waiting || !self.queue.is_empty() {
            return None;
        }
        self.waiting = true;
        Some(json!({"type":"http_stream","id":self.id,"read":true}))
    }
    fn ingest(&mut self, input: &Value) -> Result<(), &'static str> {
        self.waiting = false;
        if !self.queue.is_empty() {
            return Err("Overlapping YouTube streaming batch.");
        }
        let encoded = input["data"]
            .as_str()
            .filter(|s| s.len() <= 10924)
            .ok_or("Invalid streaming fragment.")?;
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid streaming fragment encoding.")?;
        if bytes.len() > 8192 || self.bytes.len() + bytes.len() > 512 * 1024 {
            return Err("YouTube streaming frame exceeds memory limit.");
        }
        self.bytes.extend_from_slice(&bytes);
        if input["end_message"] == true {
            let batch = proto::batch(&self.bytes)?;
            self.bytes.clear();
            self.queue = batch.items;
            self.token = batch.token;
            self.offline = batch.offline;
        }
        Ok(())
    }
}
impl YouTube {
    pub(super) fn stream_event(&mut self, input: &Value, now: u64, out: &mut Vec<Value>) {
        if input["id"] != self.stream.id || !self.stream.active {
            return;
        }
        match input["type"].as_str() {
            Some("http_stream_open") => {
                self.stream.waiting = false;
                out.push(status("connected", None));
            }
            Some("http_stream_data") => {
                if let Err(error) = self.stream.ingest(input) {
                    out.extend(self.stream.close());
                    self.authorized = false;
                    out.push(status("failed", Some(error)));
                }
            }
            Some("http_stream_closed") => {
                self.stream.active = false;
                self.stream.waiting = false;
                self.stream.bytes.clear();
                let code = input["grpc_status"].as_u64().unwrap_or(14);
                if code == 16 || input["http_status"] == 401 {
                    self.authorized = false;
                    out.push(status(
                        "needs_authorization",
                        Some("YouTube authorization expired. Reconnect the account."),
                    ));
                } else if matches!(code, 3 | 5 | 7 | 9 | 12)
                    || matches!(input["http_status"].as_u64(), Some(400 | 403 | 404))
                {
                    self.stream.blocked = true;
                    out.push(status("failed", Some("YouTube streaming chat is unavailable or access was rejected. Check the broadcast and permissions.")));
                } else {
                    let delay = (1000u64 << self.stream.failures.min(6))
                        .min(60_000)
                        .max(input["retry_after_ms"].as_u64().unwrap_or(0).min(3_600_000));
                    self.stream.failures = self.stream.failures.saturating_add(1);
                    self.stream.next = now.saturating_add(delay);
                    out.push(status(
                        "reconnecting",
                        Some("YouTube chat stream interrupted; resuming after backoff."),
                    ));
                }
            }
            _ => {}
        }
    }
    pub(super) fn drain_stream(&mut self, out: &mut Vec<Value>) {
        for _ in 0..10 {
            let Some(bytes) = self.stream.queue.pop_front() else {
                break;
            };
            match proto::message(&bytes) {
                Ok(message) => {
                    if message["snippet"]["type"] == "chatEndedEvent" {
                        self.stream.offline = true;
                    }
                    self.messages.push_back(message);
                }
                Err(error) => {
                    out.extend(self.stream.close());
                    self.authorized = false;
                    out.push(status("failed", Some(error)));
                    return;
                }
            }
        }
    }
    pub(super) fn advance_stream(&mut self, out: &mut Vec<Value>) {
        if !self.stream.queue.is_empty() || !self.messages.is_empty() {
            return;
        }
        if !self.stream.token.is_empty() {
            self.stream.page = std::mem::take(&mut self.stream.token);
            self.stream.failures = 0;
        }
        if self.stream.offline {
            out.extend(self.stream.close());
            self.chat.clear();
            self.stream.page.clear();
            self.stream.offline = false;
            out.push(status(
                "connecting",
                Some("YouTube live chat ended. Waiting for a broadcast."),
            ));
        } else {
            out.extend(self.stream.read());
        }
    }
}
