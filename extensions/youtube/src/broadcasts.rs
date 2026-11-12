use super::{YouTube, status};
use serde_json::{Value, json};

impl YouTube {
    pub(super) fn broadcast_panel(&self) -> Value {
        let mut widgets =
            vec![json!({"type":"button","id":"refresh","label":"Find live broadcasts"})];
        if self.broadcasts.len() > 1 && self.configured_video.is_empty() {
            widgets.push(json!({"type":"select","id":"select_broadcast","label":"Live broadcast","value":self.video,"options":self.broadcasts.iter().filter_map(|v|v["id"].as_str()).collect::<Vec<_>>()}));
        }
        if !self.video.is_empty() {
            widgets.push(json!({"type":"copy_text","label":"Current video ID","value":self.video}));
            if let Some(title) = self
                .broadcasts
                .iter()
                .find(|v| v["id"] == self.video)
                .and_then(|v| v["snippet"]["title"].as_str())
            {
                widgets.push(json!({"type":"label","text":title}));
            }
        }
        json!({"type":"panel","panel":{"extension_id":"youtube","title":"Live broadcast","widgets":widgets}})
    }

    fn clear_broadcast(&mut self, out: &mut Vec<Value>) {
        self.poll.reset();
        out.extend(self.stream.reset());
        self.chat.clear();
        self.video.clear();
        self.messages.clear();
        self.seen.clear();
        // Withdraw the previous channel while waiting for a new broadcast or selection.
        out.push(json!({"type":"channel","reset":true}));
    }

    pub(super) fn update_broadcasts(
        &mut self,
        value: &Value,
        input: &Value,
        now: u64,
        out: &mut Vec<Value>,
    ) {
        self.broadcasts = value["items"]
            .as_array()
            .into_iter()
            .flatten()
            .take(50)
            .filter(|v| {
                v["id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 128)
            })
            .cloned()
            .collect();
        let selected = self
            .broadcasts
            .iter()
            .find(|v| v["id"] == self.video)
            .or_else(|| (self.broadcasts.len() == 1).then(|| &self.broadcasts[0]))
            .cloned();
        if let Some(video) = selected {
            self.select_broadcast(&video, input, now, out);
        } else {
            self.clear_broadcast(out);
            out.push(status(
                "connecting",
                Some(if self.broadcasts.is_empty() {
                    "No active broadcast found. Retrying."
                } else {
                    "Choose a live broadcast to receive its chat."
                }),
            ));
        }
        out.push(self.broadcast_panel());
    }

    pub(super) fn select_broadcast(
        &mut self,
        video: &Value,
        input: &Value,
        now: u64,
        out: &mut Vec<Value>,
    ) {
        let id = video["id"].as_str().unwrap_or_default();
        let chat = video["liveStreamingDetails"]["activeLiveChatId"]
            .as_str()
            .or(video["snippet"]["liveChatId"].as_str())
            .unwrap_or_default();
        let offline = video["liveStreamingDetails"]["actualEndTime"].is_string();
        let changed = id != self.video || chat != self.chat || offline;
        if changed {
            self.clear_broadcast(out);
            self.video = id.into();
            if !offline {
                self.chat = chat.into();
            }
        }
        let enabled = |key| guest_sdk::authorization::enabled(&input["config"], key);
        if enabled("read_audience") {
            let concurrent = video["liveStreamingDetails"]["concurrentViewers"]
                .as_str()
                .and_then(|v| v.parse::<u64>().ok());
            out.push(json!({"type":"audience","sample":{"platform":"youtube","channel_id":self.video,"stream_id":self.video,"observed_at_ms":now,"concurrent":concurrent,"offline":offline,"unique":null}}));
        }
        if !self.chat.is_empty() {
            let capabilities: Vec<_> = [
                enabled("read_chat").then_some("chat.read"),
                enabled("send_chat").then_some("chat.send"),
                enabled("read_audience").then_some("audience.read"),
            ]
            .into_iter()
            .flatten()
            .collect();
            out.push(json!({"type":"channel","channel":{"id":self.video,"label":format!("YouTube / {}",video["snippet"]["title"].as_str().unwrap_or(&self.video)),"capabilities":capabilities,"constraints":{"chat.max_chars":200}}}));
            if !enabled("read_chat") {
                out.push(status("connected", None));
            } else if changed {
                out.push(status(
                    "connecting",
                    Some("Connecting to YouTube live chat…"),
                ));
            }
        } else {
            out.push(json!({"type":"channel","reset":true}));
            out.push(status(
                "connecting",
                Some("Live chat is not active. Retrying."),
            ));
        }
    }
}
