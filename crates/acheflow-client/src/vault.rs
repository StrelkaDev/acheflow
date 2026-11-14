//! Native OS credential storage; no plaintext fallback or server API.
use crate::local::LocalState;
#[cfg(target_os = "macos")]
mod native;
#[cfg(any(target_os = "macos", test))]
mod store;
#[cfg(test)]
#[path = "vault/tests.rs"]
mod workspace_tests;
pub(crate) fn key(
    server: &str,
    workspace: &str,
    provider: &str,
    application: &str,
) -> Result<String, String> {
    serde_json::to_string(&(server, workspace, provider, application))
        .map_err(|_| "Invalid local storage identity.".into())
}
#[cfg(target_os = "macos")]
pub(crate) fn load(key: &str) -> Result<Option<LocalState>, String> {
    native::with_store(|backend| store::load(backend, key))
}
#[cfg(target_os = "macos")]
pub(crate) fn save(key: &str, state: &LocalState) -> Result<(), String> {
    if state.secrets.is_empty() {
        return remove(key);
    }
    native::with_store(|backend| store::save(backend, key, state))
}
#[cfg(target_os = "macos")]
pub(crate) fn remove(key: &str) -> Result<(), String> {
    native::with_store(|backend| store::remove(backend, key))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn remove(_: &str) -> Result<(), String> {
    Err("System credential storage is not implemented for this native platform.".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn load(_: &str) -> Result<Option<LocalState>, String> {
    Ok(None)
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn save(_: &str, _: &LocalState) -> Result<(), String> {
    Err("System credential storage is not implemented for this native platform.".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use store::{Backend, LEGACY_SERVICE, WORKSPACE_SERVICE};
    #[test]
    #[ignore = "requires an unlocked macOS login keychain; run explicitly"]
    fn real_system_vault_roundtrip_isolated_record() {
        let workspace = format!(
            "test:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let key = key(
            "https://vault-test.example",
            &workspace,
            "provider-fixture",
            "application-fixture",
        )
        .unwrap();
        let second = super::key(
            "https://vault-test.example",
            &workspace,
            "other-provider",
            "application-fixture",
        )
        .unwrap();
        struct Cleanup(Vec<String>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for key in &self.0 {
                    let _ = remove(key);
                }
            }
        }
        let _cleanup = Cleanup(vec![key.clone(), second.clone()]);
        assert!(load(&key).unwrap().is_none());
        let state = LocalState {
            secrets: std::collections::BTreeMap::from([(
                "access_token".into(),
                "fake-vault-test-token".into(),
            )]),
            storage: serde_json::json!({"test":true}),
        };
        save(&key, &state).unwrap();
        // Exercise the real legacy -> workspace migration with synthetic data only.
        native::with_store(|backend| {
            backend.write(
                LEGACY_SERVICE,
                &second,
                &serde_json::to_vec(
                    &serde_json::json!({"secrets":state.secrets,"storage":state.storage}),
                )
                .unwrap(),
            )
        })
        .unwrap();
        assert_eq!(load(&second).unwrap().unwrap().secrets, state.secrets);
        native::with_store(|backend| {
            assert!(backend.read(LEGACY_SERVICE, &second)?.is_none());
            let workspace_key =
                serde_json::to_string(&("https://vault-test.example", &workspace)).unwrap();
            let bytes = backend.read(WORKSPACE_SERVICE, &workspace_key)?.unwrap();
            let container: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(container["accounts"].as_object().unwrap().len(), 2);
            Ok(())
        })
        .unwrap();
        let restored = load(&key).unwrap().unwrap();
        assert_eq!(restored.secrets, state.secrets);
        assert_eq!(restored.storage, state.storage);
        let other = super::key(
            "https://vault-test.example",
            &format!("{workspace}-other"),
            "provider-fixture",
            "application-fixture",
        )
        .unwrap();
        assert!(load(&other).unwrap().is_none());
        remove(&key).unwrap();
        assert!(load(&key).unwrap().is_none());
        assert_eq!(load(&second).unwrap().unwrap().secrets, state.secrets);
        remove(&second).unwrap();
        native::with_store(|backend| {
            let workspace_key =
                serde_json::to_string(&("https://vault-test.example", &workspace)).unwrap();
            assert!(backend.read(WORKSPACE_SERVICE, &workspace_key)?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
