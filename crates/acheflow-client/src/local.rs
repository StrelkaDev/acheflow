//! Client host primitives for local WASM panels. Provider/OAuth decisions stay in guests.
use crate::transport::{self, Secrets};
use acheflow_control::extensions::{Guest, GuestManifest};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

#[path = "local_forwarding.rs"]
mod forwarding;

/// Intentionally has no Debug or Serialize implementation.
#[derive(Default)]
pub struct LocalState {
    pub(crate) secrets: Secrets,
    pub(crate) storage: Value,
}

pub struct LocalSession {
    pub manifest: GuestManifest,
    host: acheflow_extension_api::parts::ExecutionHost,
    guest: Guest,
    pub config: BTreeMap<String, String>,
    state: LocalState,
    staged: Option<Secrets>,
    #[cfg(not(target_arch = "wasm32"))]
    vault_key: Option<String>,
    dirty: bool,
    credential_revision: u64,
    workspace_id: Option<String>,
    forwarding: forwarding::LocalForwarding,
}
impl LocalSession {
    pub fn new(
        manifest: GuestManifest,
        module: &[u8],
        config: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        Self::new_for_host(
            manifest,
            module,
            config,
            acheflow_extension_api::parts::ExecutionHost::Client,
        )
    }
    /// Ephemeral authorization for a separately selected server account. This
    /// session cannot open the device vault or execute personal device actions.
    pub fn new_server_authorization(
        manifest: GuestManifest,
        module: &[u8],
        config: BTreeMap<String, String>,
    ) -> Result<Self, String> {
        Self::new_for_host(
            manifest,
            module,
            config,
            acheflow_extension_api::parts::ExecutionHost::Server,
        )
    }
    pub(crate) fn new_for_host(
        manifest: GuestManifest,
        module: &[u8],
        config: BTreeMap<String, String>,
        host: acheflow_extension_api::parts::ExecutionHost,
    ) -> Result<Self, String> {
        manifest.require_host(host)?;
        manifest.validate()?;
        let config = manifest.settings_config(&config);
        if manifest.execution_side != acheflow_extension_api::integration::ExecutionSide::Client {
            return Err("Local panels require a client installation.".into());
        }
        let mut forwarding = forwarding::LocalForwarding::default();
        forwarding.start()?;
        Ok(Self {
            guest: Guest::load_part(module, &manifest)?,
            host,
            manifest,
            config,
            state: LocalState::default(),
            staged: None,
            #[cfg(not(target_arch = "wasm32"))]
            vault_key: None,
            dirty: false,
            credential_revision: 0,
            workspace_id: None,
            forwarding,
        })
    }
    pub fn bind_workspace(&mut self, workspace: &str) -> Result<(), String> {
        if self
            .workspace_id
            .as_deref()
            .is_some_and(|owner| owner != workspace)
        {
            return Err("Local session belongs to another workspace.".into());
        }
        if self.workspace_id.is_none() {
            // A request started before binding cannot later acquire a workspace.
            self.forwarding.cancel();
        }
        self.workspace_id = Some(workspace.into());
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn use_system_storage(&mut self, server: &str, workspace: &str) -> Result<(), String> {
        self.bind_workspace(workspace)?;
        if self.host != acheflow_extension_api::parts::ExecutionHost::Client {
            return Err("server authorization cannot open a device credential container".into());
        }
        let key = crate::vault::key(
            server,
            workspace,
            &self.manifest.id,
            self.config
                .get("client_id")
                .map(String::as_str)
                .unwrap_or_default(),
        )?;
        if self.vault_key.as_ref() == Some(&key) {
            return Ok(());
        }
        self.state = LocalState::default();
        if let Some(state) = crate::vault::load(&key)? {
            if state.secrets.len() > 32
                || state.secrets.iter().any(|(key, value)| {
                    !self.manifest.secret_fields.contains_key(key) || value.len() > 8192
                })
            {
                return Err("Invalid stored credential handles.".into());
            }
            self.state = state;
        }
        self.vault_key = Some(key);
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn use_server_workspace_storage(
        &mut self,
        server_id: &str,
        endpoint: &str,
        workspace: &str,
        workspace_name: &str,
    ) -> Result<(), String> {
        if self.host != acheflow_extension_api::parts::ExecutionHost::Client
            || !acheflow_extension_api::integration::valid_id(server_id, 128)
        {
            return Err("A verified server identity is required for device storage.".into());
        }
        let namespace = format!("client:{server_id}");
        crate::vault::name_workspace(&namespace, workspace, workspace_name, "Client")?;
        let application = self
            .config
            .get("client_id")
            .map(String::as_str)
            .unwrap_or_default();
        let old = crate::vault::key(endpoint, workspace, &self.manifest.id, application)?;
        let new = crate::vault::key(&namespace, workspace, &self.manifest.id, application)?;
        if self.vault_key.as_ref() == Some(&new) {
            return Ok(());
        }
        crate::vault::migrate(&old, &new)?;
        self.use_system_storage(&namespace, workspace)
    }
    pub fn flush_storage(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(key) = &self.vault_key {
            crate::vault::save(key, &self.state)?;
        }
        self.dirty = false;
        Ok(())
    }
    /// Committed credential version; drafts never activate provider execution.
    pub fn credential_revision(&self) -> Option<u64> {
        self.state
            .secrets
            .contains_key("access_token")
            .then_some(self.credential_revision)
    }
    pub fn account_id(&self) -> String {
        self.state.storage["account"]
            .as_str()
            .filter(|v| !v.is_empty())
            .unwrap_or("local-account")
            .to_owned()
    }
    /// Presence only; local form rendering never receives credential values.
    /// Fork credentials into an action-only instance on this device. Never bind
    /// the fork to a vault, register a collector, or send its state to a server.
    pub fn direct_action(
        &self,
        module: &[u8],
        channel: &str,
    ) -> Result<crate::direct_action::DirectAction, String> {
        self.manifest
            .require_host(acheflow_extension_api::parts::ExecutionHost::Client)?;
        if self.staged.is_some()
            || self.credential_revision().is_none()
            || self.account_id() == "local-account"
        {
            return Err("Finish authorizing this device's account first.".into());
        }
        let mut verifier = Self::new(self.manifest.clone(), module, self.config.clone())?;
        verifier.step(
            json!({"type":"local_start"}),
            &json!({"browser":cfg!(target_arch="wasm32")}),
            0,
        )?;
        for (key, value) in &self.state.secrets {
            verifier.set_secret(key, value.clone())?;
        }
        crate::direct_action::DirectAction::new(verifier, module, self.account_id(), channel)
    }
    pub(crate) fn http_context(&mut self) -> (&GuestManifest, &mut Secrets) {
        (
            &self.manifest,
            match self.staged.as_mut() {
                Some(staged) => staged,
                None => &mut self.state.secrets,
            },
        )
    }
    pub(crate) fn action_state(&self) -> Result<LocalState, String> {
        self.server_credentials_unbound()?;
        Ok(LocalState {
            secrets: self.state.secrets.clone(),
            storage: self.state.storage.clone(),
        })
    }
    pub fn has_secret(&self, key: &str) -> bool {
        self.staged
            .as_ref()
            .unwrap_or(&self.state.secrets)
            .contains_key(key)
    }
    pub fn set_secret(&mut self, key: &str, value: String) -> Result<(), String> {
        if !self.manifest.secret_fields.contains_key(key)
            || value.len() > 8192
            || value.chars().any(char::is_whitespace)
        {
            return Err("Invalid local credential input.".into());
        }
        let secrets = self
            .staged
            .get_or_insert_with(|| self.state.secrets.clone());
        if value.is_empty() {
            secrets.remove(key);
        } else {
            secrets.insert(key.into(), value);
        }
        Ok(())
    }
    pub fn capture(&mut self, payload: &mut Value, paths: &Value) -> Result<(), String> {
        let secrets = self
            .staged
            .get_or_insert_with(|| self.state.secrets.clone());
        transport::capture(
            payload,
            &serde_json::to_value(&self.manifest.secret_fields)
                .map_err(|_| "Invalid credential declaration.")?,
            secrets,
        )?;
        transport::capture(payload, paths, secrets)
    }
    pub fn step(
        &mut self,
        input: Value,
        environment: &Value,
        now: u64,
    ) -> Result<Vec<Value>, String> {
        if input["type"] == "local_forward" {
            return Err("forwarded events require a checked forwarding context".into());
        }
        self.step_inner(input, environment, now)
    }
    fn step_inner(
        &mut self,
        input: Value,
        environment: &Value,
        now: u64,
    ) -> Result<Vec<Value>, String> {
        let result = self.run_step(input, environment, now);
        if result.is_err() {
            self.stop();
        }
        result
    }
    fn run_step(
        &mut self,
        input: Value,
        environment: &Value,
        now: u64,
    ) -> Result<Vec<Value>, String> {
        if !self.forwarding.active
            && !matches!(input["type"].as_str(), Some("local_start" | "local_stop"))
        {
            return Err("Local extension session is stopped.".into());
        }
        match input["type"].as_str() {
            Some("local_start") => {
                self.forwarding.start()?;
                self.staged = None;
            }
            Some("local_stop") => {
                self.stop();
                return Ok(vec![]);
            }
            Some("local_browser") => self.forwarding.cancel(),
            _ => {}
        }
        let mut pending = VecDeque::from([input]);
        let mut output = vec![];
        for _ in 0..128 {
            let Some(mut input) = pending.pop_front() else {
                return Ok(output);
            };
            input["config"] = json!(self.config);
            input["secrets"] = json!(
                self.staged
                    .as_ref()
                    .unwrap_or(&self.state.secrets)
                    .keys()
                    .collect::<Vec<_>>()
            );
            input["storage"] = self.state.storage.clone();
            input["environment"] = environment.clone();
            input["environment"]["link"] = json!(format!(
                "{}/fwd/{}",
                acheflow_extension_api::forwarding::ForwardRecipient::Client.origin(),
                self.manifest.id
            ));
            input["now_ms"] = json!(now);
            input["execution_host"] = json!(self.host);
            input["api_version"] = json!(acheflow_extension_api::API_VERSION);
            input["workspace_id"] = json!(self.workspace_id);
            input["extension_id"] = json!(self.manifest.id);
            input["instance_id"] = json!(self.forwarding.instance_id);
            for effect in self.guest.call(input)? {
                match effect["type"].as_str() {
                    Some("crypto") => {
                        let id = effect["secret_id"].as_str().ok_or("Missing secret slot.")?;
                        if !self.manifest.secret_fields.contains_key(id) {
                            return Err("Undeclared secret slot.".into());
                        }
                        let mut bytes = [0; 32];
                        getrandom::getrandom(&mut bytes)
                            .map_err(|_| "Secure random generator unavailable.")?;
                        let verifier = URL_SAFE_NO_PAD.encode(bytes);
                        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                        self.staged
                            .get_or_insert_with(|| self.state.secrets.clone())
                            .insert(id.into(), verifier);
                        getrandom::getrandom(&mut bytes)
                            .map_err(|_| "Secure random generator unavailable.")?;
                        let nonce = URL_SAFE_NO_PAD.encode(bytes);
                        self.forwarding.issued_nonce = Some(nonce.clone());
                        pending.push_back(json!({"type":"local_crypto","id":effect["id"],"nonce":nonce,"challenge":challenge}));
                    }
                    Some("forward_wait") => self.forwarding.wait(&self.manifest, &effect, now)?,
                    Some("storage") => {
                        if serde_json::to_vec(&effect["value"])
                            .map_err(|_| "Invalid local storage.")?
                            .len()
                            > 65536
                        {
                            return Err("Local storage exceeds 64 KiB.".into());
                        }
                        self.state.storage = effect["value"].clone();
                        self.dirty = true;
                    }
                    Some("secret_begin") => {
                        self.forwarding.cancel();
                        self.staged
                            .get_or_insert_with(|| self.state.secrets.clone());
                    }
                    Some("secret_commit") => {
                        self.forwarding.cancel();
                        if let Some(mut staged) = self.staged.take() {
                            if let Some(keep) = effect["keep"].as_array() {
                                staged.retain(|key, _| keep.iter().any(|value| value == key));
                            }
                            if self.state.secrets != staged {
                                self.credential_revision += 1;
                            }
                            self.state.secrets = staged;
                            self.dirty = true;
                        }
                    }
                    Some("secret_discard") => {
                        self.forwarding.cancel();
                        self.staged = None;
                    }
                    Some("secret_clear") => {
                        self.forwarding.cancel();
                        self.credential_revision += 1;
                        self.state.secrets.clear();
                        self.staged = None;
                        self.dirty = true;
                    }
                    Some(
                        "panel" | "http" | "browser" | "browser_module" | "status" | "failure",
                    ) => output.push(effect),
                    _ => return Err("Unsupported local panel effect.".into()),
                }
            }
        }
        Err("Local event queue exceeded.".into())
    }
    pub async fn http(&mut self, effect: &Value) -> Value {
        let secrets = match self.staged.as_mut() {
            Some(staged) => staged,
            None => &mut self.state.secrets,
        };
        let mut result=transport::http(effect,&self.manifest,secrets).await.unwrap_or_else(|_|json!({"id":effect["id"],"status":0,"payload":null,"error":"Local request interrupted."}));
        result["type"] = json!("local_http");
        result
    }
    /// Explicit credential transfer; drafts and unverified accounts cannot be sent.
    pub fn server_credentials(&self, workspace: &str) -> Result<AccountCredentials, String> {
        if self.workspace_id.as_deref() != Some(workspace) {
            return Err("Account belongs to another workspace.".into());
        }
        self.server_credentials_unbound()
    }
    pub(crate) fn server_credentials_unbound(&self) -> Result<AccountCredentials, String> {
        let account = self.state.storage["account"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Authorize this account before connecting it to the server.")?;
        if self.staged.is_some() || self.credential_revision().is_none() {
            return Err("Finish account authorization before connecting it to the server.".into());
        }
        Ok(AccountCredentials {
            account_id: account.into(),
            secrets: self.state.secrets.clone(),
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn take_server_state(&mut self) -> LocalState {
        std::mem::take(&mut self.state)
    }
    /// Restores only into the matching local provider runtime, never through MCP.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install(&self, runtime: &mut crate::runtime::ProviderRuntime) -> Result<(), String> {
        if runtime.execution_host() != self.host
            || runtime.identity().connection.provider != self.manifest.id
            || self.workspace_id.as_deref() != Some(runtime.identity().workspace_id.as_str())
        {
            return Err("Provider identity mismatch.".into());
        }
        runtime.restore_local(self.state.secrets.clone(), self.state.storage.clone());
        Ok(())
    }
}

/// No Debug/Serialize: exported only by an explicit trusted-server connection.
pub struct AccountCredentials {
    pub account_id: String,
    pub secrets: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_callback_cannot_replace_an_existing_account() {
        let catalog = crate::catalog::ClientCatalog::bundled();
        let installed = &catalog.installed["twitch"];
        let mut session = LocalSession::new(
            installed.manifest.clone(),
            &installed.module,
            BTreeMap::from([("client_id".into(), "public-id".into())]),
        )
        .unwrap();
        session
            .state
            .secrets
            .insert("access_token".into(), "original".into());
        let env = json!({"browser":false,"callback_uri":"http://127.0.0.1:21941/oauth/callback"});
        session
            .step(
                json!({"type":"local_action","action_id":"connect"}),
                &env,
                1000,
            )
            .unwrap();
        let mut response = json!({"state":"wrong","access_token":"attacker","token_type":"bearer","expires_in":3600});
        session.capture(&mut response, &Value::Null).unwrap();
        assert!(response["access_token"].is_null());
        let effects = session
            .step(
                json!({"type":"local_browser","payload":response}),
                &env,
                1000,
            )
            .unwrap();
        assert!(!effects.iter().any(|e| e["type"] == "http"));
        assert_eq!(session.state.secrets["access_token"], "original");
        assert!(session.staged.is_none());
    }
}
