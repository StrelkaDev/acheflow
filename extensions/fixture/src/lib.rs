//! Real WASM contract fixture, installed only by host tests under arbitrary IDs.
use serde_json::{Value, json};
#[derive(Default)]
pub struct Fixture {
    poll: Option<(u64, usize)>,
    events: u64,
}
impl Fixture {
    pub fn handle(&mut self, input: Value) -> Vec<Value> {
        let origin = input["extension_id"].as_str().unwrap_or("fixture");
        let now = input["now_ms"].as_u64().unwrap_or(0);
        if input["config"]["visual_source_fixture"] == "true" {
            return input["sources"].as_array().into_iter().flatten().map(|source|json!({"type":"source","frame":{"source_id":source["id"],"type_id":source["kind"]["type_id"],"document":{"css":".fixture {color:white;background:blue;}","nodes":[{"tag":"div","classes":"fixture","text":origin}]}}})).collect();
        }

        if input["config"]["loop"] == "true"
            && (input["type"] == "start"
                || (input["type"] == "event" && input["name"] == "chat.message"))
        {
            self.events += 1;
            return vec![
                json!({"type":"chat","message":{"platform":origin,"channel_id":"channel","message_id":format!("{origin}:{}",self.events),"user_id":"loop","display_name":"Loop","text":"loop","roles":[],"occurred_at_ms":now}}),
            ];
        }
        match input["type"].as_str().unwrap_or_default() {
            "start" if input["config"]["studio_fixture"] == "true" => vec![
                json!({"type":"channel","channel":{"id":"channel","label":origin,"capabilities":["chat.read"]}}),
            ],
            "tick" if input["config"]["studio_fixture"] == "true" => {
                self.events += 1;
                vec![
                    json!({"type":"chat","message":{"platform":origin,"channel_id":"channel","message_id":format!("studio:{}",self.events),"user_id":"viewer","display_name":"Viewer","text":"Live fixture message","roles":[],"occurred_at_ms":now}}),
                ]
            }
            "start" if input["config"]["transport_url"].is_string() => {
                vec![json!({"type":"http","id":"slow","url":input["config"]["transport_url"]})]
            }
            "start" => vec![
                json!({"type":"channel","channel":{"id":"channel","label":origin,"capabilities":["chat.send","poll.create"]}}),
                json!({"type":"donation","donation":{"id":"receipt","channel_id":"channel","display_name":"Viewer","amount_minor":125,"currency":"EUR","message":"Thank you","occurred_at_ms":now}}),
                json!({"type":"interaction","action":"register","interaction":{"id":"wave","label":"Wave","enabled":true}}),
            ],
            "command" if input["name"] == "chat.send" => {
                let external = format!(
                    "external:{}",
                    input["request_id"].as_str().unwrap_or_default()
                );
                let sent = origin != "broken";
                vec![
                    json!({"type":"delivery","request_id":input["request_id"],"channel_id":"channel","sent":sent,"external_id":if sent{Some(external.clone())}else{None},"error":if sent{None}else{Some("Provider rejected the send")}}),
                    json!({"type":"chat","message":{"platform":origin,"channel_id":"channel","message_id":external,"user_id":"local-account","display_name":"Account","text":input["text"],"roles":[],"occurred_at_ms":now}}),
                ]
            }
            "command" if input["name"] == "poll.create" => {
                let id = input["poll_id"].as_u64().unwrap();
                let options = input["options"].as_array().unwrap().len();
                self.poll = Some((id, options));
                vec![self.update(id, options, "active", 2)]
            }
            "command" if input["name"] == "poll.close" => {
                let Some((id, options)) = self.poll.take() else {
                    return vec![];
                };
                vec![self.update(id, options, "completed", 4)]
            }
            "tick" => self
                .poll
                .map(|(id, options)| vec![self.update(id, options, "active", 3)])
                .unwrap_or_default(),
            _ => vec![],
        }
    }
}
impl Fixture {
    fn update(&self, id: u64, options: usize, state: &str, count: u64) -> Value {
        json!({"type":"poll","update":{"poll_id":id,"channel_id":"channel","external_id":format!("native-{id}"),"state":state,"votes":vec![count;options],"error":null}})
    }
}
guest_sdk::export!(Fixture);
