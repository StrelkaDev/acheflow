use serde_json::{Value, json};
#[derive(Default)]
pub struct Sample {
    count: u64,
}
impl Sample {
    pub fn handle(&mut self, input: Value) -> Vec<Value> {
        if (input["type"] == "action" && input["action_id"] == "increment")
            || (input["type"] == "forwarded" && input["action"] == "increment")
        {
            self.count += 1;
        }
        let mut out = vec![
            json!({"type":"panel","panel":{"extension_id":input["extension_id"].as_str().unwrap_or("sample"),"title":"Actions","widgets":[{"type":"metric","label":"Actions","value":self.count.to_string()},{"type":"button","id":"increment","label":"Run action"}]}}),
        ];
        if let Some(sources) = input["sources"].as_array() {
            for source in sources {
                if source["kind"]["type_id"] == "actions" {
                    out.push(json!({"type":"source","frame":{"source_id":source["id"],"type_id":"actions","document":{"css":".counter {color:white;background:#10151d;padding:16px;border-radius:12px;font-size:48px;}","nodes":[{"tag":"strong","classes":"counter","text":format!("Actions: {}",self.count)}]}}}));
                }
            }
        }
        out
    }
}
guest_sdk::export!(Sample);
