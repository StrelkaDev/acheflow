use serde_json::{Value, json};
use std::collections::VecDeque;

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
    data: Option<WireData>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}
#[derive(serde::Deserialize)]
struct WireData {
    #[serde(default)]
    chat_messages: Vec<Box<serde_json::value::RawValue>>,
    #[serde(flatten)]
    rest: serde_json::Map<String, Value>,
}

#[derive(Default)]
pub struct VkLive {
    authorization: guest_sdk::authorization::Authorization,
    channel: String,
    authorized: bool,
    channel_pending: bool,
    chat_pending: bool,
    next_channel: u64,
    next_chat: u64,
    seen: VecDeque<String>,
    messages: VecDeque<Value>,
    raw_messages: VecDeque<Box<serde_json::value::RawValue>>,
    check_gap: bool,
    overlap: bool,
    initialized: bool,
}
fn status(state: &str, error: Option<&str>) -> Value {
    json!({"type":"status","state":state,"error":error})
}
impl VkLive {
    pub fn handle_wire(&mut self, input: WireInput) -> Vec<Value> {
        let mut rest = input.rest;
        if let Some(mut payload) = input.payload {
            if let Some(mut data) = payload.data {
                if rest.get("id").is_some_and(|id| id == "chat")
                    && rest.get("status").is_some_and(|code| code == 200)
                {
                    if data.chat_messages.len() > 200 || !self.raw_messages.is_empty() {
                        return vec![status(
                            "failed",
                            Some("Unexpected VK Video Live page size or overlapping response."),
                        )];
                    }
                    self.check_gap = data.chat_messages.len() == 200 && !self.seen.is_empty();
                    self.overlap = false;
                    self.raw_messages.extend(data.chat_messages);
                    data.rest.insert("chat_messages".into(), json!([]));
                }
                payload.rest.insert("data".into(), Value::Object(data.rest));
            }
            rest.insert("payload".into(), Value::Object(payload.rest));
        }
        for _ in 0..10 {
            let Some(raw) = self.raw_messages.pop_front() else {
                break;
            };
            match serde_json::from_str::<Value>(raw.get()) {
                Ok(message) => {
                    if message["id"]
                        .as_u64()
                        .is_some_and(|id| self.seen.contains(&id.to_string()))
                    {
                        self.overlap = true;
                    }
                    self.messages.push_back(message);
                }
                Err(_) => {
                    return vec![status(
                        "failed",
                        Some("Invalid VK Video Live message JSON."),
                    )];
                }
            }
        }
        let mut out = self.handle(Value::Object(rest));
        if self.check_gap && self.raw_messages.is_empty() && self.messages.is_empty() {
            if !self.overlap {
                out.push(json!({"type":"failure","message":"VK Video Live chat history window overflowed; some messages may be missing."}));
            }
            self.check_gap = false;
        }
        out
    }
    fn http(&self, id: &str, path: &str) -> Value {
        let mut query = json!({"channel_url":self.channel});
        if id == "chat" {
            query["limit"] = json!(200);
        }
        json!({"type":"http","id":id,"url":format!("https://apidev.live.vkvideo.ru/v1/{path}"),"headers":{"Authorization":{"secret":"access_token","prefix":"Bearer "}},"query":query})
    }
    pub fn handle(&mut self, input: Value) -> Vec<Value> {
        if let Some(effects) = self
            .authorization
            .handle(&input, include_str!("../manifest.json"))
        {
            return effects;
        }
        let now = input["now_ms"].as_u64().unwrap_or(0);
        let mut out = vec![];
        match input["type"].as_str().unwrap_or_default() {
            "start" => {
                self.channel = input["config"]["channel"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .into();
                if self.channel.is_empty()
                    || self.channel.len() > 128
                    || !self
                        .channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                {
                    return vec![status(
                        "failed",
                        Some("Enter the VK Video Live channel slug, without the URL."),
                    )];
                }
                self.authorized = input["secrets"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|key| key == "access_token"));
                out.push(json!({"type":"panel","panel":{"extension_id":"vklive","title":"VK Video Live","widgets":[{"type":"label","text":"Receives chat and audience. Sending and native polls are unavailable in this version."},{"type":"button","id":"refresh","label":"Reconnect"}]}}));
                if self.authorized {
                    self.channel_pending = true;
                    out.push(self.http("channel", "channel"));
                } else {
                    out.push(status("needs_authorization", None));
                }
            }
            "action" if input["action_id"] == "refresh" => {
                self.authorized = input["secrets"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|key| key == "access_token"));
                self.next_channel = 0;
                self.next_chat = 0;
            }
            "tick" if self.authorized => {
                if now >= self.next_channel && !self.channel_pending {
                    self.channel_pending = true;
                    out.push(self.http("channel", "channel"));
                }
                if guest_sdk::authorization::enabled(&input["config"], "read_chat")
                    && self.initialized
                    && now >= self.next_chat
                    && !self.chat_pending
                    && self.messages.is_empty()
                    && self.raw_messages.is_empty()
                {
                    self.chat_pending = true;
                    out.push(self.http("chat", "chat/messages"));
                }
            }
            "http_result" => {
                let id = input["id"].as_str().unwrap_or_default();
                let code = input["status"].as_u64().unwrap_or(0);
                let data = &input["payload"]["data"];
                let retry = input["retry_after"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(30)
                    .clamp(1, 3600)
                    * 1000;
                if id == "channel" {
                    self.channel_pending = false;
                    self.next_channel = now.saturating_add(if code == 200 { 30000 } else { retry });
                    if code == 200 && data["channel"]["url"].is_string() {
                        self.initialized = true;
                        let offline = data["channel"]["status"] == "offline";
                        out.push(json!({"type":"channel","channel":{"id":self.channel,"label":format!("VK Video Live / {}",data["channel"]["nick"].as_str().unwrap_or(&self.channel)),"capabilities":[]}}));
                        if guest_sdk::authorization::enabled(&input["config"], "read_audience") {
                            out.push(json!({"type":"audience","sample":{"platform":"vklive","channel_id":self.channel,"stream_id":data["stream"]["id"],"observed_at_ms":now,"concurrent":if offline{json!(0)}else{data["stream"]["counters"]["viewers"].clone()},"offline":offline,"unique":null}}));
                        }
                        out.push(status("connected", None));
                    } else {
                        out.push(json!({"type":"failure","message":"VK Video Live channel lookup failed; retrying with backoff."}));
                    }
                } else if id == "chat" {
                    self.chat_pending = false;
                    self.next_chat = now.saturating_add(if code == 200 { 5000 } else { retry });
                    if code == 200 {
                        if let Some(messages) = data["chat_messages"].as_array() {
                            if messages.len() > 200 {
                                return vec![status(
                                    "failed",
                                    Some("VK Video Live returned more messages than requested."),
                                )];
                            }
                            // The endpoint returns a window, not a cursor. A full, disjoint
                            // window can indicate a gap, so surface it instead of hiding loss.
                            if messages.len() == 200
                                && !self.seen.is_empty()
                                && !messages.iter().any(|m| {
                                    m["id"].as_u64().is_some_and(|id| {
                                        self.seen.iter().any(|s| s.parse::<u64>() == Ok(id))
                                    })
                                })
                            {
                                out.push(json!({"type":"failure","message":"VK Video Live chat history window overflowed; some messages may be missing."}));
                            }
                            let mut ordered = messages.clone();
                            ordered.sort_by_key(|m| {
                                (
                                    m["created_at"].as_u64().unwrap_or(0),
                                    m["id"].as_u64().unwrap_or(0),
                                )
                            });
                            self.messages.extend(ordered);
                        } else {
                            out.push(json!({"type":"failure","message":"VK Video Live returned an invalid chat response."}));
                        }
                    } else {
                        out.push(json!({"type":"failure","message":"VK Video Live chat request failed; retrying with backoff."}));
                    }
                }
                if code == 401 {
                    self.authorized = false;
                    out.push(status("failed",Some("VK Video Live authorization expired. Replace the access token and enable the extension again.")));
                }
            }
            _ => {}
        }
        for _ in 0..10 {
            let Some(message) = self.messages.pop_front() else {
                break;
            };
            let Some(id) = message["id"].as_u64() else {
                continue;
            };
            let id = id.to_string();
            if self.seen.contains(&id) {
                continue;
            }
            self.seen.push_back(id.clone());
            if self.seen.len() > 2000 {
                self.seen.pop_front();
            }
            if message["is_private"] == true {
                continue;
            }
            let mut text = String::new();
            if let Some(parts) = message["parts"].as_array() {
                for part in parts {
                    if let Some(value) = part["text"]["content"]
                        .as_str()
                        .or(part["link"]["content"].as_str())
                        .or(part["link"]["url"].as_str())
                        .or(part["smile"]["name"].as_str())
                    {
                        text.push_str(value);
                    } else if let Some(nick) = part["mention"]["nick"].as_str() {
                        text.push('@');
                        text.push_str(nick);
                    }
                }
            }
            if text.is_empty() {
                continue;
            }
            let author = &message["author"];
            let roles: Vec<&str> = [("is_owner", "broadcaster"), ("is_moderator", "moderator")]
                .into_iter()
                .filter_map(|(key, role)| (author[key] == true).then_some(role))
                .collect();
            out.push(json!({"type":"chat","message":{"platform":"vklive","channel_id":self.channel,"message_id":id,"user_id":author["id"].as_u64().unwrap_or(0).to_string(),"display_name":author["nick"].as_str().unwrap_or("VK Video Live user"),"text":text,"roles":roles,"occurred_at_ms":message["created_at"].as_u64().unwrap_or(now/1000).saturating_mul(1000)}}));
        }
        out
    }
}
guest_sdk::export!(VkLive, WireInput, handle_wire);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_channel_and_chat_shapes_become_domain_events_without_duplicates() {
        let mut guest = VkLive::default();
        let effects = guest.handle(json!({"type":"start","config":{"channel":"streamer"},"secrets":["access_token"],"now_ms":1000}));
        assert!(
            effects
                .iter()
                .any(|e| e["url"] == "https://apidev.live.vkvideo.ru/v1/channel"
                    && e["query"]["channel_url"] == "streamer")
        );
        let effects = guest.handle(json!({"type":"http_result","id":"channel","status":200,"now_ms":1000,"payload":{"data":{"channel":{"url":"streamer","nick":"Streamer","status":"online"},"stream":{"id":"stream","counters":{"viewers":17}}}}}));
        assert!(
            effects
                .iter()
                .any(|e| e["type"] == "audience" && e["sample"]["concurrent"] == 17)
        );
        let message = json!({"type":"http_result","id":"chat","status":200,"now_ms":1000,"payload":{"data":{"chat_messages":[{"id":123,"created_at":42,"author":{"id":7,"nick":"Moderator","is_moderator":true},"parts":[{"text":{"content":"Hello "}},{"smile":{"name":":smile:"}},{"mention":{"nick":"Streamer"}}]}]}}});
        let effects = guest.handle(message.clone());
        let chat = effects.iter().find(|e| e["type"] == "chat").unwrap();
        assert_eq!(chat["message"]["text"], "Hello :smile:@Streamer");
        assert_eq!(chat["message"]["roles"], json!(["moderator"]));
        assert_eq!(chat["message"]["occurred_at_ms"], 42000);
        assert!(guest.handle(message).iter().all(|e| e["type"] != "chat"));
    }
}
