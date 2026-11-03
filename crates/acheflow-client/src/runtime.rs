//! Native client reactor. Dropping a pending turn cancels its HTTP/WebSocket I/O.
use crate::transport::{self, Secrets};
use acheflow_control::extensions::{Guest, GuestManifest};
use acheflow_extension_api::{client_execution::*, integration::ExecutionSide};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub struct Identity {
    pub workspace_id: String,
    pub client_id: String,
    pub generation: u64,
    pub connection: Connection,
}
/// Owns credentials, local storage and sockets for exactly one workspace/account.
/// No Debug/Serialize implementation: copying this state into server payloads is forbidden.
pub struct ProviderRuntime {
    manifest: GuestManifest,
    guest: Guest,
    identity: Identity,
    config: BTreeMap<String, String>,
    secrets: Secrets,
    storage: Value,
    socket: Option<Socket>,
    receipts: BTreeMap<String, (ClaimedInstruction, ExecutionReceipt)>,
}
impl ProviderRuntime {
    pub fn new(
        manifest: GuestManifest,
        module: &[u8],
        identity: Identity,
        config: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        manifest.validate()?;
        if manifest.execution_side != ExecutionSide::Client
            || manifest.id != identity.connection.provider
            || identity.generation == 0
        {
            return Err("invalid client execution identity".into());
        }
        let guest = Guest::load(module, manifest.capabilities.clone())?;
        Ok(Self {
            manifest,
            guest,
            identity,
            config,
            secrets: Default::default(),
            storage: Value::Null,
            socket: None,
            receipts: Default::default(),
        })
    }
    /// Used by local OAuth exchange/credential vault adapters only, never an MCP tool.
    pub fn capture_credentials(&mut self, payload: &mut Value) -> Result<(), String> {
        transport::capture(
            payload,
            &serde_json::to_value(&self.manifest.secret_fields)
                .map_err(|_| "invalid credential mapping")?,
            &mut self.secrets,
        )
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub async fn turn(&mut self, input: Value, now: u64) -> Result<Vec<Value>, String> {
        let mut pending = VecDeque::from([input]);
        let mut output = vec![];
        let mut turns = 0;
        while let Some(mut input) = pending.pop_front() {
            turns += 1;
            if turns > 512 {
                return Err("client event queue exceeded".into());
            }
            input["api_version"] = json!(acheflow_extension_api::API_VERSION);
            input["workspace_id"] = json!(self.identity.workspace_id);
            input["extension_id"] = json!(self.manifest.id);
            input["instance_id"] = json!(self.identity.generation.to_string());
            input["now_ms"] = json!(now);
            input["config"] = json!(self.config);
            input["storage"] = self.storage.clone();
            input["secrets"] = json!(self.secrets.keys().collect::<Vec<_>>());
            for effect in self.guest.call(input)? {
                match effect["type"].as_str() {
                    Some("http") => {
                        let result = transport::http(&effect, &self.manifest, &mut self.secrets).await.unwrap_or_else(|_|json!({"type":"http_result","id":effect["id"],"status":0,"payload":null,"error":"platform request interrupted"}));
                        pending.push_back(result);
                    }
                    Some("websocket") => {
                        if effect["close"] == true {
                            self.socket = None;
                            continue;
                        }
                        let result = self.websocket(&effect).await;
                        pending.push_back(
                            result.unwrap_or_else(|_| json!({"type":"websocket_closed"})),
                        );
                    }
                    Some("storage") => {
                        if serde_json::to_vec(&effect["value"])
                            .map_err(|_| "invalid storage")?
                            .len()
                            > 65536
                        {
                            return Err("local storage exceeds 64 KiB".into());
                        }
                        self.storage = effect["value"].clone();
                    }
                    // Authorization panels/status remain local. Bridge forwarding is typed.
                    Some("scene") => {
                        return Err("client provider cannot modify server scenes".into());
                    }
                    _ => output.push(effect),
                }
                if output.len() > 512 {
                    return Err("client output queue exceeded".into());
                }
            }
        }
        Ok(output)
    }
    async fn websocket(&mut self, effect: &Value) -> Result<Value, String> {
        let operation = async {
            if let Some(payload) = effect.get("send") {
                let payload = transport::resolve(payload, &self.secrets)?;
                self.socket
                    .as_mut()
                    .ok_or("socket unavailable")?
                    .send(Message::Text(
                        serde_json::to_string(&payload)
                            .map_err(|_| "invalid socket JSON")?
                            .into(),
                    ))
                    .await
                    .map_err(|_| "socket send failed")?;
                return Ok(json!({"type":"websocket_sent","id":effect["id"]}));
            }
            let url = transport::allowed_url(
                effect["url"].as_str().ok_or("missing socket URL")?,
                &self.manifest,
                true,
            )?;
            let mut request = url
                .as_str()
                .into_client_request()
                .map_err(|_| "invalid socket request")?;
            let headers = transport::resolve(&effect["headers"], &self.secrets)?;
            if let Some(headers) = headers.as_object() {
                if headers.len() > 32 {
                    return Err("too many socket headers".into());
                }
                for (name, value) in headers {
                    transport::validate_header(name)?;
                    let name = tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(
                        name.as_bytes(),
                    )
                    .map_err(|_| "invalid socket header")?;
                    let value = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(
                        value.as_str().ok_or("invalid socket header")?,
                    )
                    .map_err(|_| "invalid socket header")?;
                    request.headers_mut().insert(name, value);
                }
            }
            let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                .max_message_size(Some(1024 * 1024))
                .max_frame_size(Some(1024 * 1024));
            let (socket, _) =
                tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                    .await
                    .map_err(|_| "socket connection failed")?;
            self.socket = Some(socket);
            Ok(json!({"type":"websocket_open"}))
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), operation)
            .await
            .map_err(|_| "socket timed out".to_owned())?
    }
    /// Caller selects this future against its heartbeat, shutdown and workspace changes.
    pub async fn receive(&mut self, now: u64) -> Result<Vec<Value>, String> {
        let Some(socket) = self.socket.as_mut() else {
            std::future::pending::<()>().await;
            unreachable!()
        };
        let input = match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                json!({"type":"websocket","payload":serde_json::from_str::<Value>(&text).map_err(|_|"invalid socket JSON")?})
            }
            Some(Ok(Message::Ping(value))) => {
                socket
                    .send(Message::Pong(value))
                    .await
                    .map_err(|_| "socket pong failed")?;
                return Ok(vec![]);
            }
            Some(Ok(_)) => return Ok(vec![]),
            _ => {
                self.socket = None;
                json!({"type":"websocket_closed"})
            }
        };
        self.turn(input, now).await
    }
    fn validate_instruction(&self, claim: &ClaimedInstruction, now: u64) -> Result<(), String> {
        let i = &claim.instruction;
        let local = &self.identity;
        if (&i.workspace_id, &i.client_id, i.generation)
            != (&local.workspace_id, &local.client_id, local.generation)
        {
            return Err("instruction belongs to another client session".into());
        }
        let c = &local.connection;
        if i.connection_id != c.id || i.connection.id != c.id {
            return Err("instruction belongs to another connection".into());
        }
        if (
            &i.connection.provider,
            &i.connection.account_id,
            &i.connection.channel_id,
        ) != (&c.provider, &c.account_id, &c.channel_id)
        {
            return Err("instruction account/channel identity mismatch".into());
        }
        if i.state != InstructionState::InFlight || claim.lease == 0 {
            return Err("instruction has no execution lease".into());
        }
        if i.created_at_ms > now.saturating_add(10_000)
            || i.created_at_ms.saturating_add(INTENT_RETENTION_MS) <= now
        {
            return Err("instruction intent time is invalid".into());
        }
        if i.expires_at_ms <= now
            || i.expires_at_ms > now.saturating_add(MAX_INSTRUCTION_LIFETIME_MS)
        {
            return Err("instruction expiry is invalid".into());
        }
        let capability = &i.action.capability();
        if !c.capabilities.contains(capability) || !i.connection.capabilities.contains(capability) {
            return Err("instruction capability is not allowed locally".into());
        }
        Ok(())
    }
    /// Reserve before any I/O. Uncertain writes are never retried automatically.
    pub async fn execute(
        &mut self,
        claim: ClaimedInstruction,
        now: u64,
    ) -> Result<(ExecutionReceipt, Vec<Value>), String> {
        let i = &claim.instruction;
        if let Some((old, receipt)) = self.receipts.get(&i.id) {
            if old != &claim {
                return Err("conflicting local instruction replay".into());
            }
            return Ok((receipt.clone(), vec![]));
        }
        self.receipts.retain(|_, (claim, _)| {
            claim
                .instruction
                .created_at_ms
                .saturating_add(INTENT_RETENTION_MS)
                > now
        });
        if self.receipts.len() >= 512 {
            return Err("local instruction ledger full".into());
        }
        let authorized = self.validate_instruction(&claim, now).is_ok();
        let mut receipt = ExecutionReceipt {
            instruction_id: i.id.clone(),
            lease: claim.lease,
            outcome: ExecutionOutcome::Rejected,
            external_id: None,
        };
        let ClientAction::SendChat { text } = &i.action else {
            return Ok((receipt, vec![]));
        };
        if !authorized
            || !self.manifest.subscriptions.iter().any(|s| s == "chat.send")
            || text.trim().is_empty()
            || text.len() > 8192
        {
            return Ok((receipt, vec![]));
        }
        receipt.outcome = ExecutionOutcome::Unknown;
        self.receipts
            .insert(i.id.clone(), (claim.clone(), receipt.clone()));
        let input = json!({"type":"command","name":"chat.send","channel_id":self.identity.connection.channel_id,"request_id":i.id,"text":text});
        let remaining = std::time::Duration::from_millis(
            i.expires_at_ms
                .saturating_sub(now)
                .min(MAX_INSTRUCTION_LIFETIME_MS),
        );
        let output = match tokio::time::timeout(remaining, self.turn(input, now)).await {
            Ok(Ok(output)) => output,
            _ => return Ok((receipt, vec![])),
        };
        if let Some(effect) = output.iter().find(|e| {
            e["type"] == "delivery"
                && e["request_id"] == i.id
                && e["channel_id"] == self.identity.connection.channel_id
        }) {
            receipt.outcome = match effect["sent"].as_bool() {
                Some(true) => ExecutionOutcome::Sent,
                Some(false) => ExecutionOutcome::Failed,
                None => ExecutionOutcome::Unknown,
            };
            receipt.external_id = effect["external_id"].as_str().map(str::to_owned);
        }
        self.receipts.insert(i.id.clone(), (claim, receipt.clone()));
        Ok((receipt, output))
    }
}
