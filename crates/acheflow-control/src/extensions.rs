use serde_json::Value;
use wasmi::{
    Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

const MAX_BYTES: usize = 1024 * 1024;
const FUEL: u64 = 10_000_000;

pub use acheflow_extension_api::integration::GuestManifest;

struct Host {
    input: Value,
    input_bytes: Vec<u8>,
    output: Vec<Value>,
    output_bytes: usize,
    capabilities: Vec<String>,
    limits: StoreLimits,
}

/// A metered WASM reactor. All I/O is an emitted, capability-checked host effect.
/// The tiny template imports are format-neutral: platform field mapping lives
/// inside the guest, while the host only handles JSON pointers and typed effects.
pub struct Guest {
    store: Store<Host>,
    handle: TypedFunc<(), ()>,
}

impl Guest {
    pub fn load(bytes: &[u8], capabilities: Vec<String>) -> Result<Self, String> {
        if bytes.len() > 4 * MAX_BYTES {
            return Err("extension module exceeds 4 MiB".into());
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        let host = Host {
            input: Value::Null,
            input_bytes: vec![],
            output: vec![],
            output_bytes: 0,
            capabilities,
            limits: StoreLimitsBuilder::new()
                .memory_size(16 * MAX_BYTES)
                .table_elements(4096)
                .instances(1)
                .memories(1)
                .tables(1)
                .build(),
        };
        let mut store = Store::new(&engine, host);
        store.limiter(|host| &mut host.limits);
        store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let mut linker = Linker::new(&engine);
        linker
            .func_wrap("acheflow", "input_len", |caller: Caller<'_, Host>| -> i32 {
                caller.data().input_bytes.len() as i32
            })
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "acheflow",
                "read_input",
                |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
                    let ptr = usize::try_from(ptr)
                        .map_err(|_| wasmi::Error::new("invalid input pointer"))?;
                    let input_len = caller.data().input_bytes.len();
                    charge_host(&mut caller, input_len)?;
                    let bytes = caller.data().input_bytes.clone();
                    if usize::try_from(len).ok().is_none_or(|n| n < bytes.len()) {
                        return Err(wasmi::Error::new("input buffer too small"));
                    }
                    let memory = caller
                        .get_export("memory")
                        .and_then(|m| m.into_memory())
                        .ok_or_else(|| wasmi::Error::new("guest memory missing"))?;
                    memory
                        .write(&mut caller, ptr, &bytes)
                        .map_err(|_| wasmi::Error::new("input buffer exceeds guest memory"))?;
                    Ok(bytes.len() as i32)
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "acheflow",
                "emit_json",
                |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
                    let value = read_json(&mut caller, ptr, len)?;
                    append_effect(caller.data_mut(), value)
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "acheflow",
                "matches",
                |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
                    let condition = read_json(&mut caller, ptr, len)?;
                    let conditions = condition
                        .as_object()
                        .ok_or_else(|| wasmi::Error::new("condition must be an object"))?;
                    Ok(i32::from(conditions.iter().all(|(path, value)| {
                        let actual = caller.data().input.pointer(path);
                        if value.get("$exists") == Some(&Value::Bool(true)) {
                            actual.is_some()
                        } else {
                            actual == Some(value)
                        }
                    })))
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap(
                "acheflow",
                "emit",
                |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
                    let template = read_json(&mut caller, ptr, len)?;
                    let input_len = caller.data().input_bytes.len();
                    charge_host(&mut caller, input_len)?;
                    let mut remaining = MAX_BYTES;
                    let value = resolve(&template, &caller.data().input, 0, &mut remaining)
                        .map_err(wasmi::Error::new)?;
                    append_effect(caller.data_mut(), value)
                },
            )
            .map_err(|e| e.to_string())?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| e.to_string())?;
        let handle = instance
            .get_typed_func::<(), ()>(&store, "handle")
            .map_err(|e| e.to_string())?;
        Ok(Self { store, handle })
    }

    pub fn call(&mut self, input: Value) -> Result<Vec<Value>, String> {
        let input_bytes = serde_json::to_vec(&input).map_err(|e| e.to_string())?;
        if input_bytes.len() > MAX_BYTES {
            return Err("extension input limit reached".into());
        }
        let host = self.store.data_mut();
        host.input = input;
        host.input_bytes = input_bytes;
        host.output.clear();
        host.output_bytes = 0;
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        self.handle
            .call(&mut self.store, ())
            .map_err(|e| e.to_string())?;
        Ok(std::mem::take(&mut self.store.data_mut().output))
    }
}

fn append_effect(host: &mut Host, value: Value) -> Result<(), wasmi::Error> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| wasmi::Error::new("missing effect type"))?;
    if !host.capabilities.iter().any(|c| c == kind) {
        return Err(wasmi::Error::new("extension capability denied"));
    }
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| wasmi::Error::new("invalid effect"))?
        .len();
    if host.output.len() >= 64 || host.output_bytes.saturating_add(bytes) > MAX_BYTES {
        return Err(wasmi::Error::new("extension output limit reached"));
    }
    host.output_bytes += bytes;
    host.output.push(value);
    Ok(())
}

fn read_json(caller: &mut Caller<'_, Host>, ptr: i32, len: i32) -> Result<Value, wasmi::Error> {
    let ptr = usize::try_from(ptr).map_err(|_| wasmi::Error::new("invalid memory pointer"))?;
    let len = usize::try_from(len)
        .ok()
        .filter(|n| *n <= MAX_BYTES)
        .ok_or_else(|| wasmi::Error::new("invalid message length"))?;
    let memory = caller
        .get_export("memory")
        .and_then(|m| m.into_memory())
        .ok_or_else(|| wasmi::Error::new("guest memory missing"))?;
    charge_host(caller, len)?;
    let mut bytes = vec![0; len];
    memory
        .read(&*caller, ptr, &mut bytes)
        .map_err(|_| wasmi::Error::new("message exceeds guest memory"))?;
    serde_json::from_slice(&bytes).map_err(|_| wasmi::Error::new("invalid guest JSON"))
}

fn charge_host(caller: &mut Caller<'_, Host>, bytes: usize) -> Result<(), wasmi::Error> {
    let cost = 100 + (bytes as u64) * 8;
    let remaining = caller
        .get_fuel()?
        .checked_sub(cost)
        .ok_or_else(|| wasmi::Error::new("extension host-call fuel exhausted"))?;
    caller.set_fuel(remaining)
}
fn budget(remaining: &mut usize, bytes: usize) -> Result<(), String> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or("template output budget exceeded")?;
    Ok(())
}
fn clone_bounded(value: &Value, depth: usize, remaining: &mut usize) -> Result<Value, String> {
    if depth > 32 {
        return Err("template nesting limit".into());
    }
    budget(remaining, 32)?;
    match value {
        Value::String(s) => {
            budget(remaining, s.len())?;
            Ok(value.clone())
        }
        Value::Array(items) => items
            .iter()
            .map(|v| clone_bounded(v, depth + 1, remaining))
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                budget(remaining, k.len())?;
                Ok((k.clone(), clone_bounded(v, depth + 1, remaining)?))
            })
            .collect(),
        _ => Ok(value.clone()),
    }
}
fn resolve(
    template: &Value,
    input: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<Value, String> {
    if depth > 32 {
        return Err("template nesting limit".into());
    }
    budget(remaining, 32)?;
    match template {
        Value::String(s) if s.starts_with("$/") => clone_bounded(
            input.pointer(&s[1..]).unwrap_or(&Value::Null),
            depth + 1,
            remaining,
        ),
        Value::Array(items) => items
            .iter()
            .map(|v| resolve(v, input, depth + 1, remaining))
            .collect(),
        Value::Object(map) if map.contains_key("$map") => {
            let args = map["$map"]
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or("invalid map template")?;
            let source = resolve(&args[0], input, depth + 1, remaining)?;
            let path = args[1].as_str().ok_or("invalid map path")?;
            source
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(128)
                        .filter_map(|v| v.pointer(path))
                        .map(|v| clone_bounded(v, depth + 1, remaining))
                        .collect()
                })
                .unwrap_or_else(|| Ok(Value::Array(vec![])))
        }
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                budget(remaining, k.len())?;
                Ok((k.clone(), resolve(v, input, depth + 1, remaining)?))
            })
            .collect(),
        other => clone_bounded(other, depth, remaining),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sample_action_changes_wasm_state_and_returns_a_host_rendered_panel() {
        let mut guest = Guest::load(
            include_bytes!("../../../extensions/sample/sample.wasm"),
            vec!["panel".into()],
        )
        .unwrap();
        let start = guest.call(json!({"type":"start"})).unwrap();
        assert_eq!(start[0]["panel"]["widgets"][0]["value"], "0");
        let action = guest
            .call(json!({"type":"action","action_id":"increment"}))
            .unwrap();
        assert_eq!(action[0]["panel"]["widgets"][0]["value"], "1");
    }
    #[test]
    fn twitch_guest_normalizes_actual_eventsub_shape_and_generates_requests() {
        let manifest: GuestManifest =
            serde_json::from_str(include_str!("../../../extensions/twitch/manifest.json")).unwrap();
        manifest.validate().unwrap();
        let mut guest = Guest::load(
            include_bytes!("../../../extensions/twitch/twitch.wasm"),
            manifest.capabilities,
        )
        .unwrap();
        let effects=guest.call(json!({"type":"websocket","now_ms":1000,"payload":{"metadata":{"message_type":"notification","subscription_type":"channel.chat.message"},"payload":{"event":{"broadcaster_user_id":"b","message_id":"m","chatter_user_id":"u","chatter_user_name":"User","message":{"text":"!vote 1"},"badges":[{"set_id":"moderator","id":"1"}]}}}})).unwrap();
        let message: acheflow_extension_api::interaction::ChatMessage =
            serde_json::from_value(effects[0]["message"].clone()).unwrap();
        assert_eq!(message.text, "!vote 1");
        assert_eq!(message.roles, ["moderator"]);
        let effects = guest
            .call(json!({"type":"start","config":{"client_id":"publicapplication"}}))
            .unwrap();
        assert!(
            effects
                .iter()
                .any(|e| e["type"] == "status" && e["state"] == "needs_authorization")
        );
        let effects=guest.call(json!({"type":"action","action_id":"connect","now_ms":1000,"config":{"client_id":"publicapplication"}})).unwrap();
        assert!(
            effects
                .iter()
                .any(|e| e["url"] == "https://id.twitch.tv/oauth2/device"
                    && e["capture"]["device_code"] == "/device_code")
        );
        let effects=guest.call(json!({"type":"http_result","id":"validate","status":200,"now_ms":1000,"config":{"client_id":"publicapplication"},"payload":{"client_id":"publicapplication","user_id":"b","login":"Broadcaster","scopes":["user:read:chat","user:write:chat","channel:manage:polls"]}})).unwrap();
        assert!(
            effects
                .iter()
                .any(|e| e["type"] == "channel" && e["channel"]["id"] == "b")
        );
        let effects = guest
            .call(json!({"type":"tick","now_ms":1001,"config":{"client_id":"publicapplication"}}))
            .unwrap();
        assert!(
            effects
                .iter()
                .any(|e| e["url"] == "https://api.twitch.tv/helix/streams"
                    && e["query"]["user_id"] == "b")
        );
    }
    #[test]
    fn rust_provider_creates_refreshes_and_closes_its_native_poll() {
        let manifest: GuestManifest =
            serde_json::from_str(include_str!("../../../extensions/twitch/manifest.json")).unwrap();
        let mut guest = Guest::load(
            include_bytes!("../../../extensions/twitch/twitch.wasm"),
            manifest.capabilities,
        )
        .unwrap();
        let mut call = |mut input: Value| {
            input["config"] = json!({"client_id":"publicapplication"});
            input["now_ms"] = json!(1000);
            guest.call(input).unwrap()
        };
        call(
            json!({"type":"http_result","id":"tokens","status":200,"payload":{"expires_in":3600}}),
        );
        call(
            json!({"type":"http_result","id":"validate","status":200,"payload":{"client_id":"publicapplication","user_id":"b","login":"Broadcaster","scopes":["user:read:chat","user:write:chat","channel:manage:polls"]}}),
        );
        let create = call(
            json!({"type":"command","name":"poll.create","poll_id":7,"channel_id":"b","question":"Choose","options":["A","B"],"duration_seconds":60,"request_id":"intent"}),
        );
        assert_eq!(create[0]["method"], "POST");
        assert_eq!(create[0]["body"]["choices"][0]["title"], "A");
        let data = json!({"id":"native-7","status":"ACTIVE","choices":[{"votes":2},{"votes":1}]});
        let update = call(
            json!({"type":"http_result","id":"poll:create:7","status":200,"payload":{"data":[data]}}),
        );
        assert_eq!(update[0]["update"]["votes"], json!([2, 1]));
        let tick = call(json!({"type":"tick"}));
        assert!(tick.iter().any(|e| e["id"] == "poll:get:7"));
        let temporary =
            call(json!({"type":"http_result","id":"poll:get:7","status":503,"payload":{}}));
        assert_eq!(temporary[0]["update"]["state"], "active");
        assert_eq!(temporary[0]["update"]["votes"], json!([2, 1]));
        let close = call(
            json!({"type":"command","name":"poll.close","poll_id":7,"channel_id":"b","cancel":false}),
        );
        assert_eq!(close[0]["method"], "PATCH");
        assert_eq!(close[0]["body"]["id"], "native-7");
        let final_result = call(
            json!({"type":"http_result","id":"poll:close:7","status":200,"payload":{"data":[{"id":"native-7","status":"TERMINATED","choices":[{"votes":3},{"votes":2}]}]}}),
        );
        assert_eq!(final_result[0]["update"]["state"], "completed");
        assert_eq!(final_result[0]["update"]["votes"], json!([3, 2]));
    }
    #[test]
    fn actual_guest_maps_json_and_capabilities_gate_emitted_effects() {
        let wat = r#"(module
          (import "acheflow" "emit" (func $emit (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "{\22type\22:\22chat\22,\22message\22:\22$/value\22}")
          (func (export "handle") i32.const 0 i32.const 35 call $emit))"#;
        let mut guest = Guest::load(wat.as_bytes(), vec!["chat".into()]).unwrap();
        let result = guest.call(json!({"value":"hello"})).unwrap();
        assert_eq!(result[0]["message"], "hello");
        let mut denied = Guest::load(wat.as_bytes(), vec![]).unwrap();
        assert!(denied.call(json!({"value":"hello"})).is_err());
    }
    #[test]
    fn host_copies_and_template_expansion_have_bounded_work() {
        let mut copy = Guest::load(
            br#"(module
            (import "acheflow" "read_input" (func $read (param i32 i32) (result i32)))
            (memory (export "memory") 16)
            (func (export "handle") (loop i32.const 0 i32.const 1048576 call $read drop br 0)))"#,
            vec![],
        )
        .unwrap();
        assert!(
            copy.call(json!({"text":"x".repeat(900_000)}))
                .unwrap_err()
                .contains("host-call fuel")
        );
        let template = json!({"values":vec!["$/large";100]});
        let mut remaining = MAX_BYTES;
        assert!(
            resolve(
                &template,
                &json!({"large":"x".repeat(100_000)}),
                0,
                &mut remaining
            )
            .unwrap_err()
            .contains("output budget")
        );
    }
    #[test]
    fn fuel_memory_and_unknown_imports_are_enforced() {
        let mut infinite =
            Guest::load(br#"(module (func (export "handle") (loop br 0)))"#, vec![]).unwrap();
        assert!(infinite.call(json!({})).is_err());
        assert!(Guest::load(br#"(module (memory 300) (func (export "handle")))"#, vec![]).is_err());
        assert!(
            Guest::load(
                br#"(module (import "wasi" "network" (func)) (func (export "handle")))"#,
                vec![]
            )
            .is_err()
        );
    }
}
