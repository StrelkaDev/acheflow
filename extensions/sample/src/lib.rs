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
        vec![
            json!({"type":"panel","panel":{"extension_id":input["extension_id"].as_str().unwrap_or("sample"),"title":"Actions","widgets":[{"type":"metric","label":"Actions","value":self.count.to_string()},{"type":"button","id":"increment","label":"Run action"}]}}),
        ]
    }
}
guest_sdk::export!(Sample);
