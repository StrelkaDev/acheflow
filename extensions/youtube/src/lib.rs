use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
mod broadcasts;
mod poll;
mod proto;
mod stream;

#[derive(serde::Deserialize)]
pub struct WireInput {
    #[serde(default)]
    payload: Option<WirePayload>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(serde::Deserialize)]
struct WirePayload {
    #[serde(default)]
    items: Option<Vec<Box<serde_json::value::RawValue>>>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

#[derive(Default)]
pub struct YouTube {
    authorization: guest_sdk::authorization::Authorization,
    video: String,
    configured_video: String,
    broadcasts: Vec<Value>,
    chat: String,
    stream: stream::ChatStream,
    poll: poll::ChatPoll,
    authorized: bool,
    next_video: u64,
    video_pending: bool,
    idle_lookups: u8,
    pending: BTreeMap<String, Value>,
    seen: VecDeque<String>,
    messages: VecDeque<Value>,
}
fn status(state: &str, error: Option<&str>) -> Value {
    json!({"type":"status","state":state,"error":error})
}
fn http(id: &str, method: &str, path: &str, query: Value, body: Value) -> Value {
    json!({"type":"http","id":id,"method":method,"url":format!("https://www.googleapis.com/youtube/v3/{path}"),"headers":{"Authorization":{"secret":"access_token","prefix":"Bearer "}},"query":query,"body":body})
}
impl YouTube {
    pub fn handle_wire(&mut self, input: WireInput) -> Vec<Value> {
        let mut rest = input.rest;
        if let Some(mut payload) = input.payload {
            if let Some(items) = payload.items {
                if rest
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| self.poll.owns(id))
                    && rest.get("status") == Some(&json!(200))
                {
                    if items.len() > 2000 || !self.poll.raw_messages.is_empty() {
                        return vec![status(
                            "failed",
                            Some("Unexpected YouTube REST chat batch."),
                        )];
                    }
                    self.poll.raw_messages.extend(items);
                    payload.rest.insert("items".into(), json!([]));
                } else if rest
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("chat-poll:"))
                {
                    // A stopped/replaced account must not parse or publish late pages.
                    payload.rest.insert("items".into(), json!([]));
                } else {
                    let values = items
                        .into_iter()
                        .map(|item| serde_json::from_str(item.get()).expect("valid host JSON"))
                        .collect::<Vec<Value>>();
                    payload.rest.insert("items".into(), json!(values));
                }
            }
            rest.insert("payload".into(), Value::Object(payload.rest));
        }
        self.handle(Value::Object(rest))
    }
    fn discover(&mut self) -> Value {
        self.video_pending = true;
        if self.configured_video.is_empty() {
            http(
                "video",
                "GET",
                "liveBroadcasts",
                json!({"part":"snippet,status","broadcastStatus":"active","broadcastType":"all","maxResults":50}),
                Value::Null,
            )
        } else {
            http(
                "video",
                "GET",
                "videos",
                json!({"part":"snippet,liveStreamingDetails","id":self.configured_video}),
                Value::Null,
            )
        }
    }
    pub fn handle(&mut self, input: Value) -> Vec<Value> {
        if let Some(effects) = self
            .authorization
            .handle(&input, include_str!("../manifest.json"))
        {
            return effects;
        }
        let now = input["now_ms"].as_u64().unwrap_or(0);
        let read_chat = guest_sdk::authorization::enabled(&input["config"], "read_chat");
        let send_chat = guest_sdk::authorization::enabled(&input["config"], "send_chat");
        let mut out = vec![];
        match input["type"].as_str().unwrap_or_default() {
            "start" => {
                self.poll.reset();
                self.idle_lookups = 0;
                out.extend(self.stream.reset());
                self.messages.clear();
                self.seen.clear();
                self.video = input["config"]["video_id"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .into();
                self.configured_video = self.video.clone();
                self.chat.clear();
                self.broadcasts.clear();
                self.video_pending = false;
                self.next_video = 0;
                if !self
                    .video
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                    || self.video.len() > 128
                {
                    return vec![status(
                        "failed",
                        Some("Enter a YouTube video ID, without the URL."),
                    )];
                }
                self.authorized = input["secrets"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|s| s == "access_token"));
                out.push(self.broadcast_panel());
                if self.authorized {
                    out.push(self.discover());
                } else {
                    out.push(status("needs_authorization", None));
                }
            }
            "action" if input["action_id"] == "select_broadcast" && self.authorized => {
                if let Some(video) = self
                    .broadcasts
                    .iter()
                    .find(|v| v["id"] == input["value"])
                    .cloned()
                {
                    self.select_broadcast(&video, &input, now, &mut out);
                    out.push(self.broadcast_panel());
                }
            }
            "action" if input["action_id"] == "refresh" => {
                self.poll.reset();
                self.idle_lookups = 0;
                out.extend(self.stream.reset());
                self.authorized = input["secrets"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|s| s == "access_token"));
                if self.authorized && !self.video_pending {
                    out.push(self.discover());
                }
            }
            "tick" if self.authorized => {
                if now >= self.next_video && !self.video_pending {
                    out.push(self.discover());
                }
                if read_chat
                    && !self.chat.is_empty()
                    && !self.poll.enabled
                    && self.stream.can_open(now)
                    && self.messages.is_empty()
                {
                    out.push(self.stream.open(&self.chat));
                }
                if read_chat && !self.chat.is_empty() && self.messages.is_empty() {
                    out.extend(self.poll.request(&self.chat, now));
                }
            }
            "http_stream_open" | "http_stream_data" | "http_stream_closed" => {
                self.stream_event(&input, now, &mut out)
            }
            "stop" => {
                self.poll.reset();
                out.extend(self.stream.reset());
                self.video_pending = false;
                self.chat.clear();
                self.authorized = false;
                self.messages.clear();
            }
            "command" if input["name"] == "chat.send" => {
                let request = input["request_id"].as_str().unwrap_or_default();
                let channel = input["channel_id"].as_str().unwrap_or_default();
                if !self.authorized
                    || !send_chat
                    || self.chat.is_empty()
                    || channel != self.video
                    || self.pending.len() >= 128
                {
                    return vec![
                        json!({"type":"delivery","request_id":request,"channel_id":channel,"sent":false,"error":"YouTube live chat is unavailable"}),
                    ];
                }
                let id = format!("send:{request}");
                self.pending.insert(id.clone(), input.clone());
                out.push(http(&id,"POST","liveChat/messages",json!({"part":"snippet"}),json!({"snippet":{"liveChatId":self.chat,"type":"textMessageEvent","textMessageDetails":{"messageText":input["text"]}}})));
            }
            "http_result" => {
                let id = input["id"].as_str().unwrap_or_default();
                if id.starts_with("chat-poll:") && !self.poll.owns(id) {
                    return out;
                }
                let code = input["status"].as_u64().unwrap_or(0);
                let value = &input["payload"];
                let retry = input["retry_after"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30)
                    .clamp(1, 3600)
                    * 1000;
                if self.poll.owns(id) {
                    self.poll_result(&input, now, &mut out);
                } else if let Some(meta) = self.pending.remove(id) {
                    let sent = (200..300).contains(&code) && value["id"].is_string();
                    out.push(json!({"type":"delivery","request_id":meta["request_id"],"channel_id":meta["channel_id"],"sent":if code==0 {Value::Null}else{json!(sent)},"external_id":value["id"],"error":if sent {Value::Null}else{json!(if code==0 {"Delivery outcome is unknown"}else{"YouTube rejected the message"})}}));
                } else if id == "video" {
                    self.video_pending = false;
                    self.next_video = now.saturating_add(retry);
                    if code == 200 {
                        self.update_broadcasts(value, &input, now, &mut out);
                        if self.chat.is_empty() {
                            let delay = (60_000u64 << self.idle_lookups.min(3)).min(300_000);
                            self.idle_lookups = self.idle_lookups.saturating_add(1);
                            self.next_video = now.saturating_add(delay);
                        } else {
                            self.idle_lookups = 0;
                        }
                    } else if code != 401 {
                        out.push(json!({"type":"failure","message":"YouTube broadcast lookup failed; retrying with backoff."}));
                    }
                }
                if code == 401 {
                    self.authorized = false;
                    out.push(status("failed", Some("YouTube authorization expired. Replace the access token and enable the extension again.")));
                }
            }
            _ => {}
        }
        if !self.authorized || !read_chat {
            self.poll.reset();
            out.extend(self.stream.reset());
            self.messages.clear();
        }
        self.drain_stream(&mut out);
        self.drain_rest(&mut out);
        for _ in 0..10 {
            let Some(message) = self.messages.pop_front() else {
                break;
            };
            let id = message["id"].as_str().unwrap_or_default();
            let key = if message["snippet"]["type"] == "messageDeletedEvent" {
                format!("delete:{id}")
            } else {
                id.to_owned()
            };
            if id.is_empty() || self.seen.iter().any(|s| s == &key) {
                continue;
            }
            self.seen.push_back(key);
            if self.seen.len() > 2000 {
                self.seen.pop_front();
            }
            let snippet = &message["snippet"];
            let author = &message["authorDetails"];
            if snippet["type"] == "messageDeletedEvent" {
                out.push(json!({"type":"delete","platform":"youtube","channel_id":self.video,"message_id":snippet["messageDeletedDetails"]["deletedMessageId"],"all":false}));
            } else if let Some(text) = snippet["displayMessage"]
                .as_str()
                .or(snippet["textMessageDetails"]["messageText"].as_str())
            {
                let roles: Vec<&str> = [
                    ("isChatOwner", "broadcaster"),
                    ("isChatModerator", "moderator"),
                    ("isChatSponsor", "subscriber"),
                ]
                .into_iter()
                .filter_map(|(key, role)| (author[key] == true).then_some(role))
                .collect();
                out.push(json!({"type":"chat","message":{"platform":"youtube","channel_id":self.video,"message_id":id,"user_id":author["channelId"].as_str().unwrap_or_default(),"display_name":author["displayName"].as_str().unwrap_or("YouTube user"),"text":text,"roles":roles,"occurred_at_ms":now}}));
            }
        }
        self.advance_stream(&mut out);
        out
    }
}
guest_sdk::export!(YouTube, WireInput, handle_wire);

#[cfg(test)]
mod tests;
