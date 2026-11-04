//! All Twitch API/OAuth/EventSub knowledge stays in this independently compiled guest.
use serde_json::{Value, json};
use std::collections::BTreeMap;
const API: &str = "https://api.twitch.tv/helix/";
const SOCKET: &str = "wss://eventsub.wss.twitch.tv/ws";
#[derive(Default)]
pub struct Twitch {
    authorization: guest_sdk::authorization::Authorization,
    client_id: String,
    user: String,
    login: String,
    authed: bool,
    expires: u64,
    device_until: u64,
    next_auth: u64,
    auth_interval: u64,
    auth_pending: bool,
    refresh_pending: bool,
    next_viewers: u64,
    next_polls: u64,
    next_socket: u64,
    last_socket: u64,
    heartbeat: u64,
    resuming: bool,
    pending: BTreeMap<String, Value>,
    polls: BTreeMap<u64, Value>,
    code: String,
    uri: String,
}
fn secret(name: &str, prefix: &str) -> Value {
    json!({"secret":name,"prefix":prefix})
}
fn status(state: &str, error: Option<&str>, account: &str) -> Value {
    json!({"type":"status","state":state,"error":error,"account":account})
}
impl Twitch {
    fn panel(&self) -> Value {
        let mut widgets = vec![
            json!({"type":"button","id":"connect","label":if self.authed {"Reconnect account"}else{"Connect account"}}),
        ];
        if !self.code.is_empty() {
            widgets.push(json!({"type":"label","text":format!("Enter code: {}",self.code)}));
            widgets.push(json!({"type":"link","label":"Authorize account","url":self.uri}));
        }
        json!({"type":"panel","panel":{"extension_id":"twitch","title":"Account","widgets":widgets}})
    }
    fn http(&self, id: &str, method: &str, path: &str, body: Value) -> Value {
        json!({"type":"http","id":id,"method":method,"url":format!("{API}{path}"),"headers":{"Authorization":secret("access_token","Bearer "),"Client-Id":self.client_id},"body":body})
    }
    fn validate(&self) -> Value {
        json!({"type":"http","id":"validate","url":"https://id.twitch.tv/oauth2/validate","headers":{"Authorization":secret("access_token","OAuth ")}})
    }
    fn tokens(&self, id: &str, form: Value) -> Value {
        json!({"type":"http","id":id,"method":"POST","url":"https://id.twitch.tv/oauth2/token","form":form,"capture":{"access_token":"/access_token","refresh_token":"/refresh_token"}})
    }
    fn poll_update(
        &self,
        id: u64,
        channel: &str,
        value: &Value,
        error: Option<String>,
        options: usize,
    ) -> Value {
        let votes: Vec<u64> = value["choices"]
            .as_array()
            .map(|choices| {
                choices
                    .iter()
                    .map(|c| c["votes"].as_u64().unwrap_or(0))
                    .collect()
            })
            .unwrap_or_else(|| vec![0; options]);
        let state = if error.is_some() {
            "failed"
        } else if value["status"] == "ACTIVE" {
            "active"
        } else {
            "completed"
        };
        json!({"type":"poll","update":{"poll_id":id,"channel_id":channel,"external_id":value["id"],"state":state,"votes":votes,"error":error}})
    }
    fn error(&mut self, message: &str) -> Vec<Value> {
        self.authed = false;
        self.auth_pending = false;
        self.refresh_pending = false;
        self.device_until = 0;
        self.code.clear();
        vec![
            json!({"type":"websocket","close":true}),
            status("needs_authorization", Some(message), &self.login),
            self.panel(),
        ]
    }
    pub fn handle(&mut self, input: Value) -> Vec<Value> {
        if let Some(effects) = self
            .authorization
            .handle(&input, include_str!("../manifest.json"))
        {
            return effects;
        }
        let now = input["now_ms"].as_u64().unwrap_or(0);
        let manifest: Value =
            serde_json::from_str(include_str!("../manifest.json")).expect("bundled manifest");
        let required_scopes =
            guest_sdk::authorization::scopes(&manifest, &input["config"]).join(" ");
        let read_chat = guest_sdk::authorization::enabled(&input["config"], "read_chat");
        let read_audience = guest_sdk::authorization::enabled(&input["config"], "read_audience");
        let send_chat = guest_sdk::authorization::enabled(&input["config"], "send_chat");
        let manage_polls = guest_sdk::authorization::enabled(&input["config"], "manage_polls");
        self.client_id = input["config"]["client_id"]
            .as_str()
            .unwrap_or_default()
            .into();
        let mut out = vec![];
        match input["type"].as_str().unwrap_or_default() {
            "start" => {
                self.expires = input["storage"]["expires"].as_u64().unwrap_or(0);
                if input["secrets"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|v| v == "access_token"))
                {
                    out.push(self.validate());
                } else {
                    out.push(status("needs_authorization", None, ""));
                }
                out.push(self.panel());
            }
            "action" if input["action_id"] == "connect" => {
                if !(8..=128).contains(&self.client_id.len())
                    || !self.client_id.bytes().all(|b| b.is_ascii_alphanumeric())
                {
                    return vec![
                        json!({"type":"failure","message":"Enter a valid public application Client ID (8–128 letters or digits)."}),
                    ];
                }
                self.authed = false;
                self.device_until = 0;
                self.code.clear();
                self.polls.clear();
                self.pending.clear();
                out.push(json!({"type":"websocket","close":true}));
                out.push(status("connecting", None, ""));
                out.push(json!({"type":"http","id":"device","method":"POST","url":"https://id.twitch.tv/oauth2/device","form":{"client_id":self.client_id,"scopes":required_scopes},"capture":{"device_code":"/device_code"}}));
            }
            "http_result" => {
                let id = input["id"].as_str().unwrap_or_default();
                let code = input["status"].as_u64().unwrap_or(0);
                let value = &input["payload"];
                if id == "device" {
                    if code != 200 {
                        return self.error("The service rejected authorization. Check the public application Client ID.");
                    }
                    self.code = value["user_code"].as_str().unwrap_or_default().into();
                    self.uri = value["verification_uri"]
                        .as_str()
                        .unwrap_or_default()
                        .into();
                    self.device_until = now
                        .saturating_add(value["expires_in"].as_u64().unwrap_or(0).min(3600) * 1000);
                    self.auth_interval =
                        value["interval"].as_u64().unwrap_or(5).clamp(1, 60) * 1000;
                    self.next_auth = now + self.auth_interval;
                    self.auth_pending = false;
                    out.push(status("needs_authorization", None, ""));
                    out.push(self.panel());
                } else if matches!(id, "tokens" | "refresh") {
                    self.auth_pending = false;
                    self.refresh_pending = false;
                    if code == 200 {
                        self.expires = now.saturating_add(
                            value["expires_in"]
                                .as_u64()
                                .unwrap_or(3600)
                                .saturating_mul(1000),
                        );
                        self.device_until = 0;
                        self.code.clear();
                        out.push(json!({"type":"storage","value":{"expires":self.expires}}));
                        out.push(self.validate());
                    } else {
                        match value["message"]
                            .as_str()
                            .or(value["error"].as_str())
                            .unwrap_or_default()
                        {
                            "authorization_pending" => self.next_auth = now + self.auth_interval,
                            "slow_down" => {
                                self.auth_interval = (self.auth_interval + 5000).min(120000);
                                self.next_auth = now + self.auth_interval;
                            }
                            _ => return self.error(
                                "Authorization was denied or expired. Connect the account again.",
                            ),
                        }
                    }
                } else if id == "validate" {
                    if code != 200
                        || value["client_id"] != self.client_id
                        || !required_scopes.split_whitespace().all(|scope| {
                            value["scopes"]
                                .as_array()
                                .is_some_and(|scopes| scopes.iter().any(|s| s == scope))
                        })
                    {
                        return self.error("The account authorization is invalid or lacks the requested permissions.");
                    }
                    self.user = value["user_id"].as_str().unwrap_or_default().into();
                    self.login = value["login"].as_str().unwrap_or_default().into();
                    if self.user.is_empty() || self.user.len() > 128 {
                        return self.error("The service did not return an account identity.");
                    }
                    self.expires =
                        now.saturating_add(value["expires_in"].as_u64().unwrap_or(3600) * 1000);
                    out.push(json!({"type":"storage","value":{"expires":self.expires}}));
                    let was_authed = self.authed;
                    self.authed = true;
                    out.push(json!({"type":"channel","channel":{"id":self.user,"label":format!("Twitch / {}",self.login),"capabilities":([send_chat.then_some("chat.send"),manage_polls.then_some("poll.create")].into_iter().flatten().collect::<Vec<_>>()),"constraints":{"chat.max_chars":500,"poll.max_options":5,"poll.min_duration_seconds":15,"poll.max_duration_seconds":1800,"poll.max_question_chars":45,"poll.max_option_chars":25}}}));
                    out.push(self.panel());
                    if !was_authed && read_chat {
                        self.next_socket = 0;
                        self.last_socket = now;
                        out.push(json!({"type":"websocket","url":SOCKET}));
                        out.push(status("connecting", None, &self.login));
                    }
                } else if id == "viewers" {
                    if code == 200 {
                        let stream = &value["data"][0];
                        out.push(json!({"type":"audience","sample":{"platform":"twitch","channel_id":self.user,"stream_id":stream["id"],"observed_at_ms":now,"concurrent":stream["viewer_count"],"offline":stream.is_null(),"unique":null}}));
                    } else if matches!(code, 401 | 403) {
                        return self.error("The account authorization is no longer valid.");
                    }
                    self.next_viewers = now
                        + if code == 429 {
                            input["retry_after"]
                                .as_str()
                                .and_then(|v| v.parse::<u64>().ok())
                                .unwrap_or(30)
                                .clamp(1, 3600)
                                * 1000
                        } else {
                            30000
                        };
                } else if let Some(meta) = self.pending.remove(id) {
                    if meta["kind"] == "send" {
                        let sent = if code == 0 {
                            Value::Null
                        } else {
                            json!(code == 200 && value["data"][0]["is_sent"] == true)
                        };
                        out.push(json!({"type":"delivery","request_id":meta["request_id"],"channel_id":meta["channel_id"],"sent":sent,"external_id":value["data"][0]["message_id"],"error":if sent==true{Value::Null}else{json!(if code==0{"Delivery outcome is unknown"}else{"The service did not deliver the message"})}}));
                    } else {
                        let poll_id = meta["poll_id"].as_u64().unwrap_or(0);
                        let count = meta["options"].as_u64().unwrap_or(0) as usize;
                        let channel = meta["channel_id"].as_str().unwrap_or_default();
                        let data = &value["data"][0];
                        if code == 200 && !data.is_null() {
                            if data["status"] == "ACTIVE" {
                                let close_requested = self
                                    .polls
                                    .get(&poll_id)
                                    .and_then(|p| p.get("close_requested"))
                                    .cloned();
                                self.polls.insert(poll_id,json!({"external_id":data["id"],"channel_id":channel,"options":count,"last_value":data,"close_requested":close_requested}));
                                if id.starts_with("poll:get:")
                                    && let Some(cancel) = close_requested.filter(|v| !v.is_null())
                                {
                                    let retry = format!("poll:close:{poll_id}");
                                    if !self.pending.contains_key(&retry) {
                                        self.pending.insert(retry.clone(),json!({"kind":"poll","poll_id":poll_id,"channel_id":channel,"options":count,"external_id":data["id"]}));
                                        out.push(self.http(&retry,"PATCH","polls",json!({"broadcaster_id":channel,"id":data["id"],"status":if cancel==true{"ARCHIVED"}else{"TERMINATED"}})));
                                    }
                                }
                            } else {
                                self.polls.remove(&poll_id);
                            }
                            out.push(self.poll_update(poll_id, channel, data, None, count));
                        } else if (code == 0 || code == 429 || code >= 500)
                            && self.polls.contains_key(&poll_id)
                        {
                            let delay = input["retry_after"]
                                .as_str()
                                .and_then(|v| v.parse::<u64>().ok())
                                .unwrap_or(5)
                                .clamp(5, 3600)
                                * 1000;
                            self.next_polls = now.saturating_add(delay);
                            let cached = &self.polls[&poll_id]["last_value"];
                            let mut update =
                                self.poll_update(poll_id, channel, cached, None, count);
                            update["update"]["error"] = json!(format!(
                                "Poll update temporarily unavailable (HTTP {code}); retrying"
                            ));
                            out.push(update);
                        } else {
                            self.polls.remove(&poll_id);
                            out.push(self.poll_update(
                                poll_id,
                                channel,
                                &json!({"id":meta["external_id"]}),
                                Some(format!("Poll request failed (HTTP {code})")),
                                count,
                            ));
                        }
                    }
                } else if id.starts_with("subscribe") && code != 202 {
                    out.push(status(
                        "failed",
                        Some("The service rejected the event subscription."),
                        &self.login,
                    ));
                }
            }
            "websocket" => {
                self.last_socket = now;
                let payload = &input["payload"];
                let event = &payload["payload"]["event"];
                match payload["metadata"]["message_type"].as_str().unwrap_or_default() {
                    "session_welcome"=>{
                        self.heartbeat=payload["payload"]["session"]["keepalive_timeout_seconds"].as_u64().unwrap_or(60).clamp(5,300)*1000;self.next_socket=0;
                        if !self.resuming {for (index,kind) in ["channel.chat.message","channel.chat.message_delete","channel.chat.clear"].into_iter().enumerate(){out.push(self.http(&format!("subscribe:{index}"),"POST","eventsub/subscriptions",json!({"type":kind,"version":"1","condition":{"broadcaster_user_id":self.user,"user_id":self.user},"transport":{"method":"websocket","session_id":payload["payload"]["session"]["id"]}})));}}
                        self.resuming=false;out.push(status("running",None,&self.login));
                    }
                    "session_reconnect"=>{self.resuming=true;out.push(json!({"type":"websocket","url":payload["payload"]["session"]["reconnect_url"]}));}
                    "revocation"=>return self.error("The service revoked the event subscription. Reconnect the account."),
                    "notification"=>match payload["metadata"]["subscription_type"].as_str().unwrap_or_default() {
                        "channel.chat.message"=>out.push(json!({"type":"chat","message":{"platform":"twitch","channel_id":event["broadcaster_user_id"],"message_id":event["message_id"],"user_id":event["chatter_user_id"],"display_name":event["chatter_user_name"],"text":event["message"]["text"],"roles":event["badges"].as_array().map(|badges|badges.iter().filter_map(|b|b["set_id"].as_str()).collect::<Vec<_>>()).unwrap_or_default(),"occurred_at_ms":now}})),
                        "channel.chat.message_delete"|"channel.chat.clear"=>out.push(json!({"type":"delete","platform":"twitch","channel_id":event["broadcaster_user_id"],"message_id":event["message_id"],"all":payload["metadata"]["subscription_type"]=="channel.chat.clear"})),_=>{}
                    },_=>{}
                }
            }
            "websocket_closed" if self.authed => {
                self.next_socket = now + 5000;
                out.push(status("reconnecting", None, &self.login));
            }
            "command" => {
                let name = input["name"].as_str().unwrap_or_default();
                let channel = input["channel_id"].as_str().unwrap_or_default();
                if name == "chat.send" {
                    if !self.authed || !send_chat {
                        return vec![
                            json!({"type":"delivery","request_id":input["request_id"],"channel_id":channel,"sent":false,"external_id":null,"error":"Connect the account first"}),
                        ];
                    }
                    let id = format!("send:{}", input["request_id"].as_str().unwrap_or_default());
                    self.pending.insert(id.clone(),json!({"kind":"send","request_id":input["request_id"],"channel_id":channel}));
                    out.push(self.http(&id,"POST","chat/messages",json!({"broadcaster_id":channel,"sender_id":self.user,"message":input["text"]})));
                } else if matches!(name, "poll.create" | "poll.close") {
                    if !manage_polls {
                        return vec![self.poll_update(
                            input["poll_id"].as_u64().unwrap_or(0),
                            channel,
                            &Value::Null,
                            Some("Polls are disabled in extension settings.".into()),
                            0,
                        )];
                    }
                    let poll_id = input["poll_id"].as_u64().unwrap_or(0);
                    let options = input["options"].as_array().map(Vec::len).unwrap_or(0);
                    let seconds = input["duration_seconds"].as_u64().unwrap_or(0);
                    if name == "poll.create"
                        && (!self.authed
                            || !(2..=5).contains(&options)
                            || !(15..=1800).contains(&seconds))
                    {
                        return vec![self.poll_update(poll_id,channel,&Value::Null,Some("This service requires an authorized broadcaster, 2–5 choices and a duration of 15–1800 seconds.".into()),options)];
                    }
                    let id = format!(
                        "poll:{}:{poll_id}",
                        if name == "poll.create" {
                            "create"
                        } else {
                            "close"
                        }
                    );
                    if name == "poll.close"
                        && let Some(poll) = self.polls.get_mut(&poll_id)
                    {
                        poll["close_requested"] = input["cancel"].clone();
                    }
                    let meta = if name == "poll.create" {
                        json!({"kind":"poll","poll_id":poll_id,"channel_id":channel,"options":options})
                    } else {
                        let Some(poll) = self.polls.get(&poll_id) else {
                            return vec![];
                        };
                        json!({"kind":"poll","poll_id":poll_id,"channel_id":channel,"options":poll["options"],"external_id":poll["external_id"]})
                    };
                    let body = if name == "poll.create" {
                        json!({"broadcaster_id":channel,"title":input["question"],"choices":input["options"].as_array().unwrap().iter().map(|s|json!({"title":s})).collect::<Vec<_>>(),"duration":seconds})
                    } else {
                        json!({"broadcaster_id":channel,"id":meta["external_id"],"status":if input["cancel"]==true{"ARCHIVED"}else{"TERMINATED"}})
                    };
                    out.push(self.http(
                        &id,
                        if name == "poll.create" {
                            "POST"
                        } else {
                            "PATCH"
                        },
                        "polls",
                        body,
                    ));
                    self.pending.insert(id, meta);
                }
            }
            "tick" => {
                if self.device_until > 0 {
                    if now >= self.device_until {
                        return self
                            .error("The authorization code expired. Connect the account again.");
                    }
                    if now >= self.next_auth && !self.auth_pending {
                        self.auth_pending = true;
                        out.push(self.tokens("tokens",json!({"client_id":self.client_id,"device_code":secret("device_code",""),"grant_type":"urn:ietf:params:oauth:grant-type:device_code","scopes":required_scopes})));
                    }
                }
                if self.authed {
                    if self.expires > 0
                        && now >= self.expires.saturating_sub(60000)
                        && !self.refresh_pending
                        && input["secrets"]
                            .as_array()
                            .is_some_and(|keys| keys.iter().any(|key| key == "refresh_token"))
                    {
                        self.refresh_pending = true;
                        out.push(self.tokens("refresh",json!({"client_id":self.client_id,"refresh_token":secret("refresh_token",""),"grant_type":"refresh_token"})));
                    }
                    if read_audience && now >= self.next_viewers {
                        self.next_viewers = now + 30000;
                        let mut effect = self.http("viewers", "GET", "streams", Value::Null);
                        effect["query"] = json!({"user_id":self.user});
                        out.push(effect);
                    }
                    if read_chat && self.next_socket > 0 && now >= self.next_socket {
                        self.next_socket = 0;
                        self.resuming = false;
                        out.push(json!({"type":"websocket","url":SOCKET}));
                    } else if read_chat
                        && self.heartbeat > 0
                        && now.saturating_sub(self.last_socket) > self.heartbeat + 5000
                    {
                        self.last_socket = now;
                        self.resuming = false;
                        out.push(json!({"type":"websocket","url":SOCKET}));
                    }
                    if manage_polls && now >= self.next_polls {
                        self.next_polls = now + 5000;
                        for (id, poll) in self.polls.clone() {
                            let key = format!("poll:get:{id}");
                            if self.pending.contains_key(&key) {
                                continue;
                            }
                            let mut effect = self.http(&key, "GET", "polls", Value::Null);
                            effect["query"] = json!({"broadcaster_id":poll["channel_id"],"id":poll["external_id"]});
                            out.push(effect);
                            self.pending.insert(key,json!({"kind":"poll","poll_id":id,"channel_id":poll["channel_id"],"external_id":poll["external_id"],"options":poll["options"]}));
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }
}
guest_sdk::export!(Twitch);
