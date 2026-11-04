//! Local forwarding is scoped to one live guest and one pending callback grant.
use super::LocalSession;
use acheflow_extension_api::{
    forwarding::{ForwardLink, ForwardMode, ForwardRecipient},
    integration::GuestManifest,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::collections::BTreeMap;

struct Pending {
    field: String,
    token: String,
    deadline: u64,
}

#[derive(Default)]
pub(super) struct LocalForwarding {
    pub(super) instance_id: String,
    pub(super) issued_nonce: Option<String>,
    pub(super) active: bool,
    pending: BTreeMap<String, Pending>,
    deliveries: u64,
}
impl LocalForwarding {
    pub(super) fn start(&mut self) -> Result<(), String> {
        self.active = false;
        self.cancel();
        let mut bytes = [0; 32];
        getrandom::getrandom(&mut bytes).map_err(|_| "Secure instance identity unavailable.")?;
        self.instance_id = URL_SAFE_NO_PAD.encode(bytes);
        self.deliveries = 0;
        self.active = true;
        Ok(())
    }
    pub(super) fn cancel(&mut self) {
        self.pending.clear();
        self.issued_nonce = None;
    }
    pub(super) fn wait(
        &mut self,
        manifest: &GuestManifest,
        effect: &Value,
        now: u64,
    ) -> Result<(), String> {
        let action = effect["action"]
            .as_str()
            .ok_or("Missing forwarding action.")?;
        if !self.active
            || !manifest
                .forwarding
                .iter()
                .any(|a| a.id == action && a.mode == ForwardMode::Callback)
        {
            return Err("Undeclared or inactive callback forwarding action.".into());
        }
        let field = effect["field"]
            .as_str()
            .ok_or("Missing callback correlation field.")?;
        let token = effect["token"]
            .as_str()
            .ok_or("Missing callback correlation token.")?;
        let timeout = effect["timeout_ms"]
            .as_u64()
            .filter(|v| (1..=300_000).contains(v))
            .ok_or("Invalid callback lifetime.")?;
        if !acheflow_extension_api::integration::valid_id(field, 64)
            || self.issued_nonce.as_deref() != Some(token)
        {
            return Err("Callback correlation was not issued by this local session.".into());
        }
        let deadline = now
            .checked_add(timeout)
            .ok_or("Invalid callback deadline.")?;
        self.issued_nonce = None;
        self.pending.insert(
            action.into(),
            Pending {
                field: field.into(),
                token: token.into(),
                deadline,
            },
        );
        Ok(())
    }
    fn authorize(&mut self, action: &str, payload: &Value, now: u64) -> Result<(), String> {
        let pending = self
            .pending
            .get(action)
            .ok_or("No pending callback for this action.")?;
        if now >= pending.deadline {
            self.pending.remove(action);
            return Err("Callback expired; start a new request.".into());
        }
        if payload.get(&pending.field).and_then(Value::as_str) != Some(pending.token.as_str()) {
            return Err("Callback correlation does not match the pending request.".into());
        }
        self.pending.remove(action);
        Ok(())
    }
}

impl LocalSession {
    pub fn instance_id(&self) -> &str {
        &self.forwarding.instance_id
    }
    pub fn stop(&mut self) {
        self.forwarding.active = false;
        self.forwarding.cancel();
        self.staged = None;
    }
    pub fn forward(
        &mut self,
        workspace: &str,
        instance: &str,
        raw_url: &str,
        environment: &Value,
        now: u64,
    ) -> Result<Vec<Value>, String> {
        if !self.forwarding.active
            || self.workspace_id.as_deref() != Some(workspace)
            || self.instance_id() != instance
        {
            return Err("Forwarding context is not the active workspace and instance.".into());
        }
        let link =
            ForwardLink::parse_for(raw_url, ForwardRecipient::Client).map_err(|e| e.to_string())?;
        let mode = link.declaration(&self.manifest)?.mode;
        let mut payload = link.parameters().map_err(|e| e.to_string())?;
        if mode == ForwardMode::Callback {
            self.forwarding.authorize(&link.action, &payload, now)?;
            if let Err(error) = self.capture(&mut payload, &Value::Null) {
                self.stop();
                return Err(error);
            }
        } else if self
            .manifest
            .secret_fields
            .values()
            .any(|path| payload.pointer(path).is_some())
        {
            return Err(
                "Credential fields require a pending callback, not an ordinary event.".into(),
            );
        }
        // No raw URL/query/fragment crosses the guest boundary. Credentials are
        // staged behind handles before the extension interprets the response.
        self.forwarding.deliveries = self
            .forwarding
            .deliveries
            .checked_add(1)
            .ok_or("Forwarding request counter exhausted.")?;
        let request_id = format!("{}:{}", self.instance_id(), self.forwarding.deliveries);
        self.step_inner(
            json!({"type":"local_forward","request_id":request_id,"mode":mode,"action":link.action,"tail":link.tail,"payload":payload}),
            environment,
            now,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ClientCatalog;

    #[test]
    fn real_guest_callback_stages_captured_secret_without_committing_or_replacing_it_on_replay() {
        let catalog = ClientCatalog::bundled();
        for (id, key, parameter) in [
            ("youtube", "authorization_code", "code"),
            ("twitch", "access_token", "access_token"),
        ] {
            let installed = &catalog.installed[id];
            let mut manifest = installed.manifest.clone();
            manifest
                .forwarding
                .push(acheflow_extension_api::forwarding::ForwardAction {
                    id: "open".into(),
                    mode: ForwardMode::Event,
                });
            let mut session = LocalSession::new(
                manifest,
                &installed.module,
                BTreeMap::from([("client_id".into(), "public-id".into())]),
            )
            .unwrap();
            session.bind_workspace("workspace").unwrap();
            let environment =
                json!({"browser":false,"callback_uri":"http://127.0.0.1:21941/oauth/callback"});
            session
                .step(json!({"type":"local_start"}), &environment, 1000)
                .unwrap();
            let effects = session
                .step(
                    json!({"type":"local_action","action_id":"connect"}),
                    &environment,
                    1000,
                )
                .unwrap();
            let browser = effects.iter().find(|e| e["type"] == "browser").unwrap();
            let url = url::Url::parse(browser["url"].as_str().unwrap()).unwrap();
            let state = url
                .query_pairs()
                .find(|(k, _)| k == "state")
                .unwrap()
                .1
                .into_owned();
            let address = format!(
                "https://client.acheflow.com/fwd/{id}/oauth/callback?state={state}&{parameter}=captured-value&token_type=bearer&expires_in=3600"
            );
            let instance = session.instance_id().to_owned();
            let effects = session
                .forward("workspace", &instance, &address, &environment, 1001)
                .unwrap();
            assert_eq!(
                session
                    .staged
                    .as_ref()
                    .unwrap()
                    .get(key)
                    .map(String::as_str),
                Some("captured-value")
            );
            assert!(!session.state.secrets.contains_key(key));
            assert!(
                !serde_json::to_string(&effects)
                    .unwrap()
                    .contains("captured-value")
            );
            let unsolicited = format!(
                "https://client.acheflow.com/fwd/{id}/open?{parameter}=event-injected-value"
            );
            assert!(
                session
                    .forward("workspace", &instance, &unsolicited, &environment, 1002)
                    .is_err(),
                "an ordinary event must not stage credentials"
            );
            assert_eq!(
                session
                    .staged
                    .as_ref()
                    .unwrap()
                    .get(key)
                    .map(String::as_str),
                Some("captured-value")
            );
            let replay = address.replace("captured-value", "replacement-value");
            assert!(
                session
                    .forward("workspace", &instance, &replay, &environment, 1002)
                    .is_err()
            );
            assert_eq!(
                session
                    .staged
                    .as_ref()
                    .unwrap()
                    .get(key)
                    .map(String::as_str),
                Some("captured-value")
            );
            session.stop();
            assert!(session.staged.is_none());
        }
    }
}
