//! One-shot device-local writes. Shared chat always comes from the server.
use crate::{
    local::LocalSession,
    transport::{self, Secrets},
};
use acheflow_control::extensions::{Guest, GuestManifest};
use acheflow_extension_api::client_execution::ExecutionOutcome;
use serde_json::{Value, json};
use std::{collections::VecDeque, future::Future, pin::Pin};

pub trait ActionHttp {
    fn request<'a>(
        &'a mut self,
        effect: &'a Value,
        manifest: &'a GuestManifest,
        secrets: &'a mut Secrets,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + 'a>>;
}
pub struct PlatformHttp;
impl ActionHttp for PlatformHttp {
    fn request<'a>(
        &'a mut self,
        effect: &'a Value,
        manifest: &'a GuestManifest,
        secrets: &'a mut Secrets,
    ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + 'a>> {
        Box::pin(transport::http(effect, manifest, secrets))
    }
}
/// No Debug/Serialize: this instance owns a private copy of device credentials.
pub struct DirectAction {
    verifier: LocalSession,
    guest: Guest,
    account: String,
    channel: String,
    secrets: Secrets,
    storage: Value,
    prepared: bool,
    used: bool,
    write_attempted: bool,
    max_chars: usize,
}
impl DirectAction {
    pub(crate) fn new(
        verifier: LocalSession,
        module: &[u8],
        account: String,
        channel: &str,
    ) -> Result<Self, String> {
        if !acheflow_extension_api::integration::valid_id(channel, 128)
            || !verifier.manifest.allows_action("chat.send")
            || !verifier
                .manifest
                .subscriptions
                .iter()
                .any(|s| s == "chat.send")
        {
            return Err("This client part cannot send to the selected channel.".into());
        }
        let guest = Guest::load_part(module, &verifier.manifest)?;
        Ok(Self {
            verifier,
            guest,
            account,
            channel: channel.into(),
            secrets: Default::default(),
            storage: Value::Null,
            prepared: false,
            used: false,
            write_attempted: false,
            max_chars: 8192,
        })
    }
    pub fn account_id(&self) -> &str {
        &self.account
    }
    pub async fn prepare(&mut self, http: &mut impl ActionHttp) -> Result<(), String> {
        if self.used || self.prepared {
            return Err("This action instance cannot be prepared twice.".into());
        }
        let environment = json!({"browser":cfg!(target_arch="wasm32")});
        let effects = self.verifier.step(
            json!({"type":"local_server_authorize"}),
            &environment,
            now(),
        )?;
        let mut pending = VecDeque::from(effects);
        let mut requests = 0;
        for _ in 0..64 {
            let Some(effect) = pending.pop_front() else {
                break;
            };
            match effect["type"].as_str() {
                Some("panel" | "status") => {}
                Some("http") if requests < 8 => {
                    read_only(&effect)?;
                    requests += 1;
                    let (manifest, secrets) = self.verifier.http_context();
                    let mut response = http
                        .request(&effect, manifest, secrets)
                        .await
                        .map_err(|_| "Account verification could not reach the platform.")?;
                    response["type"] = json!("local_http");
                    pending.extend(self.verifier.step(response, &environment, now())?);
                }
                _ => return Err("Account verification requested an unsupported operation.".into()),
            }
        }
        if !pending.is_empty() {
            return Err("Account verification exceeded its limit.".into());
        }
        let credentials = self.verifier.server_credentials_unbound().map_err(|_| {
            "Reconnect this device’s account and approve sending permission.".to_owned()
        })?;
        if credentials.account_id != self.account {
            return Err(
                "The authorized platform account changed. Reconnect this device's account.".into(),
            );
        }
        let state = self.verifier.action_state()?;
        self.secrets = state.secrets;
        self.storage = state.storage;
        let output = self
            .read_turn(
                json!({"type":"prepare_action","channel_id":self.channel}),
                http,
            )
            .await?;
        let channel = output
            .iter()
            .find(|e| e["type"] == "channel" && e["channel"]["id"] == self.channel)
            .map(|e| &e["channel"])
            .ok_or("The platform could not resolve the selected channel.")?;
        if !channel["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == "chat.send"))
        {
            return Err("This device's platform account cannot send to that channel.".into());
        }
        self.max_chars = channel["constraints"]["chat.max_chars"]
            .as_u64()
            .unwrap_or(8192)
            .min(8192) as usize;
        self.prepared = true;
        Ok(())
    }
    fn input(&self, mut value: Value) -> Value {
        value["api_version"] = json!(acheflow_extension_api::API_VERSION);
        value["execution_host"] = json!("client");
        value["extension_id"] = json!(self.verifier.manifest.id);
        value["config"] = json!(self.verifier.config);
        value["now_ms"] = json!(now());
        value["storage"] = self.storage.clone();
        value["secrets"] = json!(self.secrets.keys().collect::<Vec<_>>());
        value
    }
    async fn read_turn(
        &mut self,
        input: Value,
        http: &mut impl ActionHttp,
    ) -> Result<Vec<Value>, String> {
        let mut pending = VecDeque::from([input]);
        let mut output = Vec::new();
        let mut requests = 0;
        for _ in 0..64 {
            let Some(input) = pending.pop_front() else {
                return Ok(output);
            };
            let input = self.input(input);
            for effect in self.guest.call(input)? {
                match effect["type"].as_str() {
                    Some("http") if requests < 8 => {
                        read_only(&effect)?;
                        requests += 1;
                        pending.push_back(
                            http.request(&effect, &self.verifier.manifest, &mut self.secrets)
                                .await
                                .map_err(|_| "Channel preparation could not reach the platform.")?,
                        );
                    }
                    Some("storage") => {
                        if serde_json::to_vec(&effect["value"])
                            .map_err(|_| "Invalid private action storage.")?
                            .len()
                            > 65536
                        {
                            return Err("Action storage exceeds limit.".into());
                        }
                        self.storage = effect["value"].clone();
                    }
                    Some("channel" | "panel" | "status" | "failure") => output.push(effect),
                    Some("websocket" | "http_stream") if effect["close"] == true => {}
                    _ => {
                        return Err(
                            "Subscriptions and writes are forbidden during action preparation."
                                .into(),
                        );
                    }
                }
            }
        }
        Err("Channel preparation exceeded its limit.".into())
    }
    /// The caller checks current Acheflow permission, workspace and account
    /// revision immediately before the sole external write. This instance cannot
    /// retry, including after a timeout or a lost receipt.
    pub async fn send<F, Fut>(
        &mut self,
        request_id: &str,
        text: &str,
        http: &mut impl ActionHttp,
        authorize: F,
    ) -> Result<ExecutionOutcome, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let result = self.send_inner(request_id, text, http, authorize).await;
        match result {
            Err(_) if self.write_attempted => Ok(ExecutionOutcome::Unknown),
            other => other,
        }
    }
    async fn send_inner<F, Fut>(
        &mut self,
        request_id: &str,
        text: &str,
        http: &mut impl ActionHttp,
        authorize: F,
    ) -> Result<ExecutionOutcome, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        if !self.prepared || self.used {
            return Err("Prepare a new action instance before sending.".into());
        }
        self.used = true;
        if text.trim().is_empty() || text.len() > 8192 || text.chars().count() > self.max_chars {
            return Err("The message exceeds this channel's text limit.".into());
        }
        let mut permission = Some(authorize);
        let mut writes = 0;
        let mut requests = 0;
        let mut pending = VecDeque::from([
            json!({"type":"command","name":"chat.send","channel_id":self.channel,"request_id":request_id,"text":text}),
        ]);
        let deadline = web_time::Instant::now() + std::time::Duration::from_secs(30);
        let mut outcome = ExecutionOutcome::Unknown;
        for _ in 0..64 {
            let Some(input) = pending.pop_front() else {
                return Ok(if writes == 0 {
                    ExecutionOutcome::Rejected
                } else {
                    outcome
                });
            };
            if web_time::Instant::now() > deadline {
                return Ok(ExecutionOutcome::Unknown);
            }
            let input = self.input(input);
            let effects = match self.guest.call(input) {
                Ok(effects) => effects,
                Err(error) => {
                    return if writes > 0 {
                        Ok(ExecutionOutcome::Unknown)
                    } else {
                        Err(error)
                    };
                }
            };
            for effect in effects {
                match effect["type"].as_str() {
                    Some("http") if requests < 8 => {
                        requests += 1;
                        let method = effect["method"].as_str().unwrap_or("GET");
                        transport::allowed_url(
                            effect["url"].as_str().ok_or("Missing platform URL.")?,
                            &self.verifier.manifest,
                            false,
                        )?;
                        if !matches!(method, "GET" | "HEAD") {
                            if writes != 0 {
                                return Ok(ExecutionOutcome::Unknown);
                            }
                            let authorize = permission
                                .take()
                                .ok_or("Action permission was already consumed.")?;
                            authorize().await?;
                            writes = 1;
                            self.write_attempted = true;
                        }
                        match http
                            .request(&effect, &self.verifier.manifest, &mut self.secrets)
                            .await
                        {
                            Ok(response) => pending.push_back(response),
                            Err(_) => return Ok(ExecutionOutcome::Unknown),
                        }
                    }
                    Some("delivery")
                        if effect["request_id"] == request_id
                            && effect["channel_id"] == self.channel =>
                    {
                        outcome = match effect["sent"].as_bool() {
                            Some(true) => ExecutionOutcome::Sent,
                            Some(false) => ExecutionOutcome::Failed,
                            None => ExecutionOutcome::Unknown,
                        }
                    }
                    Some(
                        "panel" | "status" | "failure" | "channel" | "chat" | "audience" | "delete"
                        | "poll",
                    ) => {}
                    Some("storage") => {}
                    _ => return Ok(ExecutionOutcome::Unknown),
                }
            }
        }
        Ok(ExecutionOutcome::Unknown)
    }
}
fn read_only(effect: &Value) -> Result<(), String> {
    if !matches!(effect["method"].as_str().unwrap_or("GET"), "GET" | "HEAD") {
        return Err("Writes are forbidden before action authorization.".into());
    }
    Ok(())
}
fn now() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now().max(0.) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
#[cfg(test)]
mod tests;
