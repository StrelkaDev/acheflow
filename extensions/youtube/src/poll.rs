//! REST recovery when the advertised gRPC service is unavailable.
use super::{YouTube, http, status};
use serde_json::{Value, json};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct ChatPoll {
    pub enabled: bool,
    next: u64,
    pending: String,
    page: String,
    blocked: bool,
    serial: u64,
    pub raw_messages: VecDeque<Box<serde_json::value::RawValue>>,
}
impl ChatPoll {
    pub fn reset(&mut self) {
        *self = Self {
            serial: self.serial,
            ..Default::default()
        };
    }
    pub fn owns(&self, id: &str) -> bool {
        !self.pending.is_empty() && self.pending == id
    }
    pub fn request(&mut self, chat: &str, now: u64) -> Option<Value> {
        if !self.enabled
            || self.blocked
            || !self.pending.is_empty()
            || !self.raw_messages.is_empty()
            || now < self.next
        {
            return None;
        }
        self.serial += 1;
        self.pending = format!("chat-poll:{}", self.serial);
        let mut query = json!({"part":"id,snippet,authorDetails","liveChatId":chat,"maxResults":200,
                "fields":"items(id,snippet(type,displayMessage,textMessageDetails/messageText,messageDeletedDetails/deletedMessageId),authorDetails(channelId,displayName,isChatOwner,isChatSponsor,isChatModerator)),nextPageToken,pollingIntervalMillis,offlineAt"});
        if !self.page.is_empty() {
            query["pageToken"] = json!(self.page);
        }
        Some(http(
            &self.pending,
            "GET",
            "liveChat/messages",
            query,
            Value::Null,
        ))
    }
}

impl YouTube {
    pub(super) fn drain_rest(&mut self, out: &mut Vec<Value>) {
        for _ in 0..10 {
            let Some(raw) = self.poll.raw_messages.pop_front() else {
                break;
            };
            match serde_json::from_str(raw.get()) {
                Ok(message) => self.messages.push_back(message),
                Err(_) => {
                    self.poll.blocked = true;
                    self.poll.raw_messages.clear();
                    out.push(status("failed", Some("Invalid YouTube REST message.")));
                    return;
                }
            }
        }
    }
    pub(super) fn poll_result(&mut self, input: &Value, now: u64, out: &mut Vec<Value>) {
        self.poll.pending.clear();
        let code = input["status"].as_u64().unwrap_or(0);
        let value = &input["payload"];
        let delay = input["retry_after"]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30)
            .clamp(1, 3600)
            * 1000;
        self.poll.next = now.saturating_add(delay);
        if code == 200 {
            let items = value["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let page = value["nextPageToken"].as_str().unwrap_or_default();
            if (!value["items"].is_null() && !value["items"].is_array())
                || items.len() > 2000
                || page.len() > 8192
            {
                self.poll.blocked = true;
                self.poll.raw_messages.clear();
                out.push(status("failed", Some("Invalid YouTube chat response.")));
                return;
            }
            self.poll.page = page.into();
            self.poll.next = now.saturating_add(
                value["pollingIntervalMillis"]
                    .as_u64()
                    .unwrap_or(5000)
                    .max(1000),
            );
            self.messages.extend(items.iter().cloned());
            if value["offlineAt"].is_string() {
                self.poll.blocked = true;
                out.push(status(
                    "connecting",
                    Some("YouTube live chat ended. Waiting for a broadcast."),
                ));
            } else {
                out.push(status(
                    "connected",
                    Some("Receiving YouTube chat through REST; streaming service unavailable."),
                ));
            }
        } else if matches!(code, 400 | 403 | 404) {
            self.poll.blocked = true;
            out.push(status("failed",Some(&format!("YouTube REST chat rejected the request (HTTP {code}). Check broadcast access and API quota; then find live broadcasts again."))));
        } else if code != 401 {
            out.push(status(
                "reconnecting",
                Some("YouTube REST chat request failed; retrying after backoff."),
            ));
        }
    }
}
