//! Extension-side OAuth state machine. The client supplies only bounded primitives.
use serde_json::{Value, json};

#[path = "application_setup.rs"]
mod application_setup;

#[derive(Default)]
pub struct Authorization {
    state: String,
    pending: bool,
    pending_until: u64,
    expires: u64,
    account: String,
    error: String,
    authorized: bool,
}
pub fn enabled(config: &Value, id: &str) -> bool {
    config[id].as_str().is_none_or(|v| v == "true")
}
pub fn scopes(manifest: &Value, config: &Value) -> Vec<String> {
    let mut scopes = Vec::new();
    if let Some(permissions) = manifest["authentication"]["permissions"].as_array() {
        for permission in permissions {
            let any = permission["any"].as_array();
            let requested = any.is_none_or(|ids| {
                ids.is_empty()
                    || ids
                        .iter()
                        .any(|id| enabled(config, id.as_str().unwrap_or_default()))
            });
            let excluded = permission["unless"].as_array().is_some_and(|ids| {
                ids.iter()
                    .any(|id| enabled(config, id.as_str().unwrap_or_default()))
            });
            if requested
                && !excluded
                && let Some(scope) = permission["scope"].as_str()
            {
                scopes.push(scope.into());
            }
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn query(url: &str, fields: Vec<(&str, String)>) -> String {
    format!(
        "{url}?{}",
        fields
            .into_iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(&v)))
            .collect::<Vec<_>>()
            .join("&")
    )
}
fn secret(id: &str, prefix: &str) -> Value {
    json!({"secret":id,"prefix":prefix})
}
impl Authorization {
    fn panel(&self, manifest: &Value, input: &Value) -> Value {
        let auth = &manifest["authentication"];
        let permissions = scopes(manifest, &input["config"]);
        let setup = application_setup::widgets(manifest, input);
        let mut fields =
            vec![json!({"type":"secret_input","id":"access_token","label":"Access token"})];
        if input["environment"]["browser"] != true && auth["native_pkce"] == true {
            fields.push(json!({"type":"secret_input","id":"refresh_token","label":"Refresh token (optional)"}));
        }
        let mut widgets = vec![
            json!({"type":"label","text":if permissions.is_empty(){"Requested permissions: no privileged scopes.".into()}else{format!("Requested permissions: {}",permissions.join(", "))}}),
            json!({"type":"row","widgets":[{"type":"button","id":"connect","label":format!("Connect with {}",manifest["name"].as_str().unwrap_or("account")),"primary":true}]}),
            json!({"type":"disclosure","label":"Enter tokens manually","widgets":[
                {"type":"row","widgets":fields},
                {"type":"row","widgets":[{"type":"button","id":"use_tokens","label":"Use tokens"},{"type":"button","id":"disconnect","label":"Disconnect account"}]}
            ]}),
            json!({"type":"disclosure","label":"Application setup","widgets":setup}),
        ];
        if self.pending {
            widgets
                .push(json!({"type":"label","text":"Waiting for authorization in the browser…"}));
        }
        if self.authorized {
            widgets.push(json!({"type":"label","text":format!("Authorized locally{}",if self.account.is_empty(){String::new()}else{format!(" as {}",self.account)})}));
        }
        if !self.error.is_empty() {
            widgets.push(json!({"type":"label","text":self.error}));
        }
        json!({"type":"panel","panel":{"extension_id":manifest["id"],"title":"Account authorization","widgets":widgets}})
    }
    fn validate(&self, manifest: &Value, input: &Value) -> Value {
        let validation = &manifest["authentication"]["validation"];
        let mut effect = validation["request"].clone();
        effect["type"] = json!("http");
        effect["id"] = json!("auth:validate");
        if let Some(headers) = validation["request"]["headers"].as_object() {
            for (key, value) in headers {
                if let Some(setting) = value["config"].as_str() {
                    effect["headers"][key] =
                        json!(input["config"][setting].as_str().unwrap_or_default());
                }
            }
        }
        effect
    }
    fn fail(&mut self, message: &str) {
        self.authorized = false;
        self.pending = false;
        self.error = message.into();
    }
    fn refresh(&self, manifest: &Value, input: &Value) -> Option<Vec<Value>> {
        let auth = &manifest["authentication"];
        if input["environment"]["browser"] == true
            || !auth["token_url"].as_str().is_some_and(|s| !s.is_empty())
            || !input["secrets"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key == "refresh_token"))
        {
            return None;
        }
        let mut form = json!({"grant_type":"refresh_token","client_id":input["config"]["client_id"],"refresh_token":secret("refresh_token","")});
        if input["secrets"]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|key| key == "client_secret"))
        {
            form["client_secret"] = secret("client_secret", "");
        }
        Some(vec![
            json!({"type":"secret_begin"}),
            json!({"type":"http","id":"auth:refresh","method":"POST","url":auth["token_url"],"form":form,"capture":manifest["secret_fields"]}),
        ])
    }
    fn granted(&self, manifest: &Value, input: &Value, payload: &Value, path: &str) -> bool {
        let Some(granted) = payload.pointer(path) else {
            return scopes(manifest, &input["config"]).is_empty();
        };
        let granted: Vec<&str> = if let Some(array) = granted.as_array() {
            array.iter().filter_map(Value::as_str).collect()
        } else {
            granted
                .as_str()
                .unwrap_or_default()
                .split([' ', ','])
                .collect()
        };
        scopes(manifest, &input["config"]).iter().all(|scope| {
            granted.contains(&scope.as_str())
                || manifest["authentication"]["scope_implies"][scope]
                    .as_array()
                    .is_some_and(|alternatives| {
                        alternatives
                            .iter()
                            .any(|s| granted.contains(&s.as_str().unwrap_or_default()))
                    })
        })
    }
    pub fn handle(&mut self, input: &Value, manifest_text: &str) -> Option<Vec<Value>> {
        let kind = input["type"].as_str().unwrap_or_default();
        if !kind.starts_with("local_") {
            return None;
        }
        let manifest: Value =
            serde_json::from_str(manifest_text).expect("bundled extension manifest");
        let now = input["now_ms"].as_u64().unwrap_or(0);
        let mut output = vec![];
        if self.pending && now > self.pending_until {
            self.fail("Authorization expired. Connect again.");
            return Some(vec![
                json!({"type":"secret_discard"}),
                self.panel(&manifest, input),
            ]);
        }
        match kind {
            "local_start" => {
                self.pending = false;
                self.authorized = false;
                self.error.clear();
                self.expires = input["storage"]["expires"].as_u64().unwrap_or(0);
                if input["secrets"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|s| s == "access_token"))
                {
                    if self.expires > 0
                        && now >= self.expires.saturating_sub(60_000)
                        && let Some(refresh) = self.refresh(&manifest, input)
                    {
                        output.extend(refresh);
                    } else {
                        output.push(json!({"type":"secret_begin"}));
                        output.push(self.validate(&manifest, input));
                    }
                }
            }
            "local_action" => self.local_action(&manifest, input, now, &mut output),
            "local_crypto" if self.pending => self.local_crypto(&manifest, input, &mut output),
            "local_browser" if self.pending => {
                self.local_browser(&manifest, input, now, &mut output)
            }
            "local_forward"
                if self.pending
                    && input["mode"] == "callback"
                    && input["action"] == "oauth"
                    && input["tail"] == "callback" =>
            {
                self.local_browser(&manifest, input, now, &mut output)
            }
            "local_http" => self.local_http(&manifest, input, now, &mut output),
            "local_tick"
                if self.authorized
                    && self.expires > 0
                    && now >= self.expires.saturating_sub(60_000) =>
            {
                if let Some(refresh) = self.refresh(&manifest, input) {
                    output.extend(refresh);
                } else if now >= self.expires {
                    self.fail("Authorization expired. Connect again or enter a new token.");
                }
            }
            _ => {}
        }
        if !self.error.is_empty() {
            output.push(json!({"type":"secret_discard"}));
        }
        output.push(self.panel(&manifest, input));
        Some(output)
    }
    fn local_action(&mut self, manifest: &Value, input: &Value, now: u64, output: &mut Vec<Value>) {
        match input["action_id"].as_str().unwrap_or_default() {
            "connect" => {
                let client = input["config"]["client_id"].as_str().unwrap_or_default();
                if client.is_empty()
                    || client.len() > 256
                    || client.chars().any(char::is_whitespace)
                {
                    self.fail("Enter the application's public Client ID first.");
                } else if let Some(message) = application_setup::missing_secret(manifest, input) {
                    self.fail(&message);
                } else {
                    self.pending = true;
                    self.authorized = false;
                    self.error.clear();
                    self.pending_until = now.saturating_add(300_000);
                    output.push(json!({"type":"secret_begin"}));
                    output.push(
                        json!({"type":"crypto","id":"auth:random","secret_id":"code_verifier"}),
                    );
                }
            }
            "use_tokens" => {
                self.error.clear();
                output.push(json!({"type":"secret_begin"}));
                output.push(self.validate(manifest, input));
            }
            "disconnect" => {
                *self = Self::default();
                output.push(json!({"type":"secret_clear"}));
                output.push(json!({"type":"storage","value":{}}));
            }
            _ => {}
        }
    }
    fn local_crypto(&mut self, manifest: &Value, input: &Value, output: &mut Vec<Value>) {
        let auth = &manifest["authentication"];

        self.state = input["nonce"].as_str().unwrap_or_default().into();
        if self.state.len() < 32 {
            self.fail("Secure authorization could not be initialized.");
        } else if input["environment"]["browser"] == true
            && auth["browser_module"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        {
            output.push(json!({"type":"browser_module","id":"auth:browser","arguments":{"client_id":input["config"]["client_id"],"scopes":scopes(manifest,&input["config"])},"capture":manifest["secret_fields"]}));
        } else {
            let pkce = auth["native_pkce"] == true;
            let redirect = input["environment"]["callback_uri"]
                .as_str()
                .unwrap_or_default();
            let mut fields = vec![
                (
                    "client_id",
                    input["config"]["client_id"]
                        .as_str()
                        .unwrap_or_default()
                        .into(),
                ),
                ("redirect_uri", redirect.into()),
                ("response_type", if pkce { "code" } else { "token" }.into()),
                ("state", self.state.clone()),
            ];
            let scopes = scopes(manifest, &input["config"]);
            if !scopes.is_empty() {
                fields.push((
                    "scope",
                    scopes.join(auth["scope_separator"].as_str().unwrap_or(" ")),
                ));
            }
            if pkce {
                fields.extend([
                    (
                        "code_challenge",
                        input["challenge"].as_str().unwrap_or_default().into(),
                    ),
                    ("code_challenge_method", "S256".into()),
                    ("access_type", "offline".into()),
                    ("prompt", "consent".into()),
                ]);
            }
            if manifest["forwarding"].as_array().is_some_and(|actions| {
                actions
                    .iter()
                    .any(|a| a["id"] == "oauth" && a["mode"] == "callback")
            }) {
                output.push(json!({"type":"forward_wait","action":"oauth","field":"state","token":self.state,"timeout_ms":300_000}));
            }
            output.push(json!({"type":"browser","id":"auth:browser","url":query(auth["authorize_url"].as_str().unwrap_or_default(),fields),"capture":manifest["secret_fields"]}));
        }
    }
    fn local_browser(
        &mut self,
        manifest: &Value,
        input: &Value,
        now: u64,
        output: &mut Vec<Value>,
    ) {
        let auth = &manifest["authentication"];

        let payload = &input["payload"];
        let sdk = input["environment"]["browser"] == true
            && auth["browser_module"]
                .as_str()
                .is_some_and(|s| !s.is_empty());
        if input["error"].is_string()
            || payload["error"].is_string()
            || (!sdk && payload["state"] != self.state)
        {
            self.fail("Authorization denied, cancelled or state mismatch. Connect again.");
        } else if auth["native_pkce"] == true && !sdk {
            let mut form = json!({"grant_type":"authorization_code","client_id":input["config"]["client_id"],"redirect_uri":input["environment"]["callback_uri"],"code":secret("authorization_code",""),"code_verifier":secret("code_verifier","")});
            if input["secrets"]
                .as_array()
                .is_some_and(|s| s.iter().any(|s| s == "client_secret"))
            {
                form["client_secret"] = secret("client_secret", "");
            }
            output.push(json!({"type":"http","id":"auth:token","method":"POST","url":auth["token_url"],"form":form,"capture":manifest["secret_fields"]}));
        } else {
            output.extend(self.token_result(manifest, input, payload, now));
        }
    }
    fn local_http(&mut self, manifest: &Value, input: &Value, now: u64, output: &mut Vec<Value>) {
        let auth = &manifest["authentication"];

        if input["status"].as_u64() != Some(200) {
            self.fail("The platform rejected authorization. Check application setup, token and requested permissions.");
        } else if input["id"] == "auth:token" || input["id"] == "auth:refresh" {
            output.extend(self.token_result(manifest, input, &input["payload"], now));
        } else if input["id"] == "auth:validate" {
            let payload = &input["payload"];
            let validation = &auth["validation"];
            let scope_path = validation["scope_path"].as_str().unwrap_or("/scope");
            let client_path = validation["client_path"].as_str();
            let client = input["config"]["client_id"].as_str().unwrap_or_default();
            if !self.granted(manifest, input, payload, scope_path)
                || client_path.is_some_and(|path| {
                    !client.is_empty()
                        && payload.pointer(path).and_then(Value::as_str) != Some(client)
                })
            {
                self.fail("The token belongs to another application or lacks permissions. Reconnect after updating settings.");
            } else {
                self.authorized = true;
                self.pending = false;
                self.error.clear();
                output.push(json!({"type":"secret_commit","keep":["access_token","refresh_token","client_secret"]}));
                self.account = validation["account_path"]
                    .as_str()
                    .and_then(|p| payload.pointer(p))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into();
                if let Some(seconds) = validation["expires_path"]
                    .as_str()
                    .and_then(|p| payload.pointer(p))
                    .and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    })
                {
                    self.expires = now.saturating_add(seconds.min(31_536_000) * 1000);
                }
                output.push(json!({"type":"storage","value":{"expires":self.expires,"scopes":scopes(manifest,&input["config"]),"account":self.account}}));
            }
        }
    }
    fn token_result(
        &mut self,
        manifest: &Value,
        input: &Value,
        payload: &Value,
        now: u64,
    ) -> Vec<Value> {
        if payload["token_type"]
            .as_str()
            .is_none_or(|s| !s.eq_ignore_ascii_case("bearer"))
        {
            self.fail("Invalid token response.");
            return vec![];
        }
        let number = |key: &str| {
            payload[key]
                .as_u64()
                .or_else(|| payload[key].as_str().and_then(|s| s.parse::<u64>().ok()))
        };
        self.expires = number("expires_in")
            .map(|seconds| now.saturating_add(seconds.min(31_536_000) * 1000))
            .or_else(|| number("expire_time").map(|unix| unix.saturating_mul(1000)))
            .unwrap_or(0);
        if self.expires <= now {
            self.fail("Missing or expired authorization response.");
            return vec![];
        }
        vec![self.validate(manifest, input)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_saved_native_token_is_refreshed_before_validation() {
        let manifest = r#"{"id":"fixture","name":"Fixture","authentication":{"token_url":"https://provider.example/token","permissions":[],"validation":{"request":{"url":"https://provider.example/validate"}}},"secret_fields":{"access_token":"/access_token","refresh_token":"/refresh_token"}}"#;
        let mut auth = Authorization::default();
        let input = json!({"type":"local_start","now_ms":10000,"storage":{"expires":1},"secrets":["access_token","refresh_token"],"config":{"client_id":"app"},"environment":{"browser":false}});
        let effects = auth.handle(&input, manifest).unwrap();
        assert!(effects.iter().any(|e| e["id"] == "auth:refresh"));
        assert!(!effects.iter().any(|e| e["id"] == "auth:validate"));
        let mut input = input;
        input["environment"]["browser"] = json!(true);
        let effects = auth.handle(&input, manifest).unwrap();
        assert!(
            !effects.iter().any(|e| e["id"] == "auth:refresh"),
            "browser never uses refresh credentials"
        );
    }
    #[test]
    fn state_mismatch_never_exchanges_code_or_validates_a_token() {
        let manifest = r#"{"id":"fixture","name":"Fixture","authentication":{"authorize_url":"https://provider.example/authorize","permissions":[]},"secret_fields":{}}"#;
        let mut auth = Authorization::default();
        auth.handle(
            &json!({"type":"local_action","action_id":"connect","config":{"client_id":"public"}}),
            manifest,
        )
        .unwrap();
        let output=auth.handle(&json!({"type":"local_browser","payload":{"state":"attacker","token_type":"bearer","expires_in":3600}}),manifest).unwrap();
        assert!(output.iter().all(|e| e["type"] != "http"));
        assert!(!auth.authorized);
    }
}
