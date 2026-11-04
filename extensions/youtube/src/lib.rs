use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

/// Decode the minimum 200-message API page as raw items. Parsing every nested
/// message in one WASM call would exhaust the sandbox's fixed fuel allowance.
#[derive(serde::Deserialize)]
pub struct WireInput {
    #[serde(default)]
    payload: Option<WirePayload>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(Default, serde::Deserialize)]
struct WirePayload {
    #[serde(default)]
    items: Vec<Box<serde_json::value::RawValue>>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

#[derive(Default)]
pub struct YouTube {
    authorization: guest_sdk::authorization::Authorization,
    video: String,
    chat: String,
    page: String,
    authorized: bool,
    next_video: u64,
    next_chat: u64,
    video_pending: bool,
    chat_pending: bool,
    pending: BTreeMap<String, Value>,
    seen: VecDeque<String>,
    messages: VecDeque<Value>,
    raw_messages: VecDeque<Box<serde_json::value::RawValue>>,
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
            if rest.get("id").is_some_and(|id| id == "chat")
                && rest.get("status").is_some_and(|code| code == 200)
            {
                if payload.items.len() > 200 || !self.raw_messages.is_empty() {
                    return vec![status(
                        "failed",
                        Some("Unexpected YouTube chat page size or overlapping response."),
                    )];
                }
                self.raw_messages.extend(payload.items);
                payload.rest.insert("items".into(), json!([]));
            } else if !payload.items.is_empty() {
                payload.rest.insert(
                    "items".into(),
                    Value::Array(
                        payload
                            .items
                            .into_iter()
                            .filter_map(|raw| serde_json::from_str(raw.get()).ok())
                            .collect(),
                    ),
                );
            }
            rest.insert("payload".into(), Value::Object(payload.rest));
        }
        for _ in 0..10 {
            let Some(raw) = self.raw_messages.pop_front() else {
                break;
            };
            match serde_json::from_str(raw.get()) {
                Ok(message) => self.messages.push_back(message),
                Err(_) => return vec![status("failed", Some("Invalid YouTube message JSON."))],
            }
        }
        self.handle(Value::Object(rest))
    }
    fn discover(&mut self) -> Value {
        self.video_pending = true;
        if self.video.is_empty() {
            http(
                "video",
                "GET",
                "liveBroadcasts",
                json!({"part":"snippet,status","broadcastStatus":"active","broadcastType":"all"}),
                Value::Null,
            )
        } else {
            http(
                "video",
                "GET",
                "videos",
                json!({"part":"snippet,liveStreamingDetails","id":self.video}),
                Value::Null,
            )
        }
    }
    pub fn handle(&mut self, mut input: Value) -> Vec<Value> {
        if let Some(effects) = self
            .authorization
            .handle(&input, include_str!("../manifest.json"))
        {
            return effects;
        }
        let now = input["now_ms"].as_u64().unwrap_or(0);
        let read_chat = guest_sdk::authorization::enabled(&input["config"], "read_chat");
        let read_audience = guest_sdk::authorization::enabled(&input["config"], "read_audience");
        let send_chat = guest_sdk::authorization::enabled(&input["config"], "send_chat");
        let mut out = vec![];
        match input["type"].as_str().unwrap_or_default() {
            "start" => {
                self.video = input["config"]["video_id"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .into();
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
                out.push(json!({"type":"panel","panel":{"extension_id":"youtube","title":"YouTube","widgets":[{"type":"button","id":"refresh","label":"Find live broadcast"},{"type":"label","text":"Authorize the account for the functions enabled in extension settings."}]}}));
                if self.authorized {
                    out.push(self.discover());
                } else {
                    out.push(status("needs_authorization", None));
                }
            }
            "action" if input["action_id"] == "refresh" => {
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
                    && now >= self.next_chat
                    && !self.chat_pending
                    && self.messages.is_empty()
                    && self.raw_messages.is_empty()
                {
                    self.chat_pending = true;
                    let mut query = json!({"part":"snippet,authorDetails","liveChatId":self.chat,"maxResults":200});
                    if !self.page.is_empty() {
                        query["pageToken"] = json!(self.page);
                    }
                    out.push(http("chat", "GET", "liveChat/messages", query, Value::Null));
                }
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
                let code = input["status"].as_u64().unwrap_or(0);
                let value = &input["payload"];
                let retry = input["retry_after"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30)
                    .clamp(1, 3600)
                    * 1000;
                if let Some(meta) = self.pending.remove(id) {
                    let sent = (200..300).contains(&code) && value["id"].is_string();
                    out.push(json!({"type":"delivery","request_id":meta["request_id"],"channel_id":meta["channel_id"],"sent":if code==0 {Value::Null}else{json!(sent)},"external_id":value["id"],"error":if sent {Value::Null}else{json!(if code==0 {"Delivery outcome is unknown"}else{"YouTube rejected the message"})}}));
                } else if id == "video" {
                    self.video_pending = false;
                    self.next_video = now.saturating_add(retry);
                    if code == 200 {
                        let video = &value["items"][0];
                        if video.is_null() {
                            self.chat.clear();
                            out.push(status(
                                "connecting",
                                Some("No active broadcast found. Retrying."),
                            ));
                        } else {
                            self.video = video["id"].as_str().unwrap_or_default().into();
                            let chat = video["liveStreamingDetails"]["activeLiveChatId"]
                                .as_str()
                                .or(video["snippet"]["liveChatId"].as_str())
                                .unwrap_or_default();
                            if chat != self.chat {
                                self.page.clear();
                                self.chat = chat.into();
                            }
                            let details = &video["liveStreamingDetails"];
                            let concurrent = details["concurrentViewers"]
                                .as_str()
                                .and_then(|s| s.parse::<u64>().ok());
                            let offline = details["actualEndTime"].is_string();
                            if read_audience {
                                out.push(json!({"type":"audience","sample":{"platform":"youtube","channel_id":self.video,"stream_id":self.video,"observed_at_ms":now,"concurrent":concurrent,"offline":offline,"unique":null}}));
                            }
                            if !self.chat.is_empty() {
                                out.push(json!({"type":"channel","channel":{"id":self.video,"label":format!("YouTube / {}",video["snippet"]["title"].as_str().unwrap_or(&self.video)),"capabilities":if send_chat {vec!["chat.send"]}else{vec![]},"constraints":{"chat.max_chars":200}}}));
                                out.push(status("connected", None));
                            } else {
                                out.push(status(
                                    "connecting",
                                    Some("Live chat is not active. Retrying."),
                                ));
                            }
                        }
                    } else if code != 401 {
                        out.push(json!({"type":"failure","message":"YouTube broadcast lookup failed; retrying with backoff."}));
                    }
                } else if id == "chat" {
                    self.chat_pending = false;
                    self.next_chat = now.saturating_add(if code == 200 {
                        value["pollingIntervalMillis"]
                            .as_u64()
                            .unwrap_or(5000)
                            .max(1000)
                    } else {
                        retry
                    });
                    if code == 200 {
                        self.page = value["nextPageToken"].as_str().unwrap_or_default().into();
                        let offline = value["offlineAt"].is_string();
                        if let Value::Array(items) = input["payload"]["items"].take() {
                            if items.len() > 200 {
                                return vec![status(
                                    "failed",
                                    Some("YouTube returned more messages than requested."),
                                )];
                            }
                            self.messages.extend(items);
                        }
                        if offline {
                            self.chat.clear();
                            self.page.clear();
                        }
                    } else {
                        out.push(json!({"type":"failure","message":"YouTube chat request failed; retrying with backoff."}));
                    }
                }
                if code == 401 {
                    self.authorized = false;
                    out.push(status("failed", Some("YouTube authorization expired. Replace the access token and enable the extension again.")));
                }
            }
            _ => {}
        }
        for _ in 0..10 {
            let Some(message) = self.messages.pop_front() else {
                break;
            };
            let id = message["id"].as_str().unwrap_or_default();
            if id.is_empty() || self.seen.iter().any(|s| s == id) {
                continue;
            }
            self.seen.push_back(id.into());
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
        out
    }
}
guest_sdk::export!(YouTube, WireInput, handle_wire);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_live_chat_and_respects_server_polling_interval() {
        let mut guest = YouTube::default();
        let input = |kind: &str| json!({"type":kind,"config":{"video_id":"video"},"secrets":["access_token"],"now_ms":1000});
        let effects = guest.handle(input("start"));
        assert!(
            effects
                .iter()
                .any(|e| e["url"] == "https://www.googleapis.com/youtube/v3/videos")
        );
        let effects = guest.handle(json!({"type":"http_result","id":"video","status":200,"now_ms":1000,"payload":{"items":[{"id":"video","snippet":{"title":"Live"},"liveStreamingDetails":{"activeLiveChatId":"chat","concurrentViewers":"42","actualStartTime":"2026-11-02T10:00:00Z"}}]}}));
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
        let effects = guest.handle(input("tick"));
        assert!(
            effects
                .iter()
                .any(|e| e["id"] == "chat" && e["query"]["liveChatId"] == "chat")
        );
        guest.handle(json!({"type":"http_result","id":"chat","status":200,"now_ms":1000,"payload":{"pollingIntervalMillis":12000,"nextPageToken":"next+/=","items":[]}}));
        let mut tick = input("tick");
        tick["now_ms"] = json!(12999);
        assert!(!guest.handle(tick.clone()).iter().any(|e| e["id"] == "chat"));
        tick["now_ms"] = json!(13000);
        assert!(
            guest
                .handle(tick)
                .iter()
                .any(|e| e["query"]["pageToken"] == "next+/=")
        );
    }
}
