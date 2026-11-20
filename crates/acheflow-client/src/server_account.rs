//! Dedicated server account namespace; it never loads a device's local account.
pub mod authorization;
use crate::{local::LocalState, runtime::ProviderRuntime};
use std::sync::{Arc, Mutex};
pub struct ServerAccount {
    key: String,
    state: Arc<Mutex<LocalState>>,
    scope: (String, String, String),
    last_checkpoint: Option<std::time::Instant>,
}
impl ServerAccount {
    pub fn name_workspace(server: &str, workspace: &str, name: &str) -> Result<(), String> {
        crate::vault::name_workspace(&format!("server:{server}"), workspace, name, "Server")
    }
    pub fn remove(
        server: &str,
        workspace: &str,
        provider: &str,
        slot: &str,
        account: &str,
    ) -> Result<(), String> {
        crate::vault::remove(&storage_key(server, workspace, provider, slot, account)?)
    }
    pub fn remove_personal(
        server: &str,
        workspace: &str,
        user: &str,
        provider: &str,
        slot: &str,
        account: &str,
    ) -> Result<(), String> {
        crate::vault::remove(&personal_storage_key(
            server, workspace, user, provider, slot, account,
        )?)
    }
    pub fn load(
        server: &str,
        workspace: &str,
        provider: &str,
        application: &str,
        account: &str,
    ) -> Result<Option<Self>, String> {
        let key = storage_key(server, workspace, provider, application, account)?;
        crate::vault::load(&key).map(|state| {
            state.map(|state| Self {
                key,
                state: Arc::new(Mutex::new(state)),
                scope: (workspace.into(), provider.into(), account.into()),
                last_checkpoint: None,
            })
        })
    }
    fn check(&self, runtime: &ProviderRuntime) -> Result<(), String> {
        if runtime.execution_host() != acheflow_extension_api::parts::ExecutionHost::Server
            || self.scope
                != (
                    runtime.identity().workspace_id.clone(),
                    runtime.identity().connection.provider.clone(),
                    runtime.identity().connection.account_id.clone(),
                )
        {
            return Err("Server account belongs to another binding.".into());
        }
        Ok(())
    }
    pub fn install(&self, runtime: &mut ProviderRuntime) -> Result<(), String> {
        self.check(runtime)?;
        let state = self
            .state
            .lock()
            .map_err(|_| "Account storage unavailable.")?;
        runtime.restore_server(state.secrets.clone(), state.storage.clone())?;
        drop(state);
        let (key, scope, cache) = (self.key.clone(), self.scope.clone(), self.state.clone());
        runtime.set_account_sink(Box::new(move |identity, secrets, storage| {
            if scope
                != (
                    identity.workspace_id.clone(),
                    identity.connection.provider.clone(),
                    identity.connection.account_id.clone(),
                )
            {
                return Err("Account binding changed.".into());
            }
            let mut previous = cache.lock().map_err(|_| "Account storage unavailable.")?;
            if previous.secrets != *secrets {
                let next = LocalState {
                    secrets: secrets.clone(),
                    storage: storage.clone(),
                };
                crate::vault::save(&key, &next)?;
                *previous = next;
            }
            Ok(())
        }));
        Ok(())
    }
    pub fn checkpoint(&mut self, runtime: &ProviderRuntime) -> Result<(), String> {
        self.checkpoint_with(runtime, crate::vault::save)
    }
    fn checkpoint_with(
        &mut self,
        runtime: &ProviderRuntime,
        save: impl FnOnce(&str, &LocalState) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check(runtime)?;
        if !runtime.account_storage_available() {
            return Err("account_storage_unavailable".into());
        }
        let next = runtime.account_state();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Account storage unavailable.")?;
        let credentials_changed = next.secrets != state.secrets;
        let storage_due = self
            .last_checkpoint
            .is_none_or(|time| time.elapsed() >= std::time::Duration::from_secs(1));
        if credentials_changed || (next.storage != state.storage && storage_due) {
            save(&self.key, &next)?;
            *state = next;
            self.last_checkpoint = Some(std::time::Instant::now());
        }
        Ok(())
    }
}

pub(crate) fn storage_key(
    server: &str,
    workspace: &str,
    provider: &str,
    application: &str,
    account: &str,
) -> Result<String, String> {
    let slot = serde_json::to_string(&(application, account))
        .map_err(|_| "Invalid server account identity.")?;
    crate::vault::key(&format!("server:{server}"), workspace, provider, &slot)
}
pub(super) fn personal_storage_key(
    server: &str,
    workspace: &str,
    user: &str,
    provider: &str,
    slot: &str,
    account: &str,
) -> Result<String, String> {
    // A tuple of four fields cannot collide with a shared account's two-field slot,
    // even when an Apply caller chooses arbitrary public application strings.
    let slot = serde_json::to_string(&("personal", user, slot, account))
        .map_err(|_| "Invalid personal account identity.")?;
    crate::vault::key(&format!("server:{server}"), workspace, provider, &slot)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Identity;
    use acheflow_extension_api::client_execution::Connection;
    use serde_json::json;
    #[test]
    fn personal_slots_never_collide_with_other_users_or_public_shared_bindings() {
        let personal = personal_storage_key(
            "server",
            "workspace",
            "alice",
            "provider",
            "slot",
            "account",
        )
        .unwrap();
        assert_ne!(
            personal,
            personal_storage_key("server", "workspace", "bob", "provider", "slot", "account")
                .unwrap()
        );
        assert_ne!(
            personal,
            personal_storage_key(
                "server",
                "other-workspace",
                "alice",
                "provider",
                "slot",
                "account"
            )
            .unwrap()
        );
        let crafted = serde_json::to_string(&("personal", "alice", "slot", "account")).unwrap();
        assert_ne!(
            personal,
            storage_key("server", "workspace", "provider", &crafted, "account").unwrap()
        );
        assert_ne!(
            personal,
            storage_key("server", "workspace", "provider", "slot", "account").unwrap()
        );
    }
    #[test]
    fn renewed_credentials_checkpoint_atomically_and_never_cross_bindings() {
        let manifest=serde_json::from_value(json!({"id":"alpha","name":"Alpha","api_version":2,"module":"fixture.wasm","execution_side":"client","services":[],"capabilities":["channel","chat","delivery"],"secret_fields":{"access_token":"/token"}})).unwrap();
        let identity = Identity {
            workspace_id: "workspace".into(),
            client_id: "server-actor".into(),
            generation: 1,
            connection: Connection {
                channel_label: None,
                stream_id: None,
                id: "binding".into(),
                provider: "alpha".into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: Default::default(),
            },
        };
        let mut runtime = ProviderRuntime::new_for_host(
            manifest,
            include_bytes!("../../acheflow-server/tests/fixtures/domain.wasm"),
            identity,
            Default::default(),
            acheflow_extension_api::parts::ExecutionHost::Server,
        )
        .unwrap();
        let mut account = ServerAccount {
            key: "fixture".into(),
            scope: ("workspace".into(), "alpha".into(), "bot".into()),
            last_checkpoint: None,
            state: Arc::new(Mutex::new(LocalState::default())),
        };
        let mut payload = json!({"token":"synthetic-renewed-token"});
        runtime.capture_credentials(&mut payload).unwrap();
        assert!(payload["token"].is_null());
        assert!(
            account
                .checkpoint_with(&runtime, |_, _| Err("locked".into()))
                .is_err()
        );
        assert!(account.state.lock().unwrap().secrets.is_empty());
        account
            .checkpoint_with(&runtime, |_, state| {
                assert_eq!(state.secrets["access_token"], "synthetic-renewed-token");
                Ok(())
            })
            .unwrap();
        assert_eq!(
            account.state.lock().unwrap().secrets["access_token"],
            "synthetic-renewed-token"
        );
        account.scope.2 = "another-account".into();
        assert!(account.install(&mut runtime).is_err());
        assert!(
            account
                .checkpoint_with(&runtime, |_, _| panic!("wrong binding must not be saved"))
                .is_err()
        );
        runtime.set_account_sink(Box::new(|_, _, _| Err("locked".into())));
        let mut renewal = json!({"token":"next-synthetic-token"});
        assert!(runtime.capture_credentials(&mut renewal).is_err());
        assert!(!runtime.account_storage_available());
    }
}
