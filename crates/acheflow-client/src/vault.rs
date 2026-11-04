//! Native OS credential storage; no plaintext fallback or server API.
use crate::local::LocalState;
#[cfg(target_os = "macos")]
const SERVICE: &str = "Acheflow client extension storage v1";
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
    let bytes = match security_framework::passwords::get_generic_password(SERVICE, key) {
        Ok(bytes) => bytes,
        Err(error) if error.code() == -25300 => return Ok(None),
        Err(_) => return Err("System credential storage could not be opened.".into()),
    };
    if bytes.len() > 256 * 1024 {
        return Err("Stored extension data exceeds limit.".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid local credential record.")?;
    let secrets = serde_json::from_value(value["secrets"].clone())
        .map_err(|_| "Invalid local credential record.")?;
    Ok(Some(LocalState {
        secrets,
        storage: value["storage"].clone(),
    }))
}
#[cfg(target_os = "macos")]
pub(crate) fn save(key: &str, state: &LocalState) -> Result<(), String> {
    if state.secrets.is_empty() {
        return remove(key);
    }
    let bytes =
        serde_json::to_vec(&serde_json::json!({"secrets":state.secrets,"storage":state.storage}))
            .map_err(|_| "Could not encode local credential record.")?;
    if bytes.len() > 256 * 1024 {
        return Err("Extension credential record exceeds limit.".into());
    }
    security_framework::passwords::set_generic_password(SERVICE, key, &bytes)
        .map_err(|_| "System credential storage could not save this account.".into())
}
#[cfg(target_os = "macos")]
pub(crate) fn remove(key: &str) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(SERVICE, key) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()),
        Err(_) => Err("System credential storage could not remove this account.".into()),
    }
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
    #[test]
    #[ignore = "requires an unlocked macOS login keychain; run explicitly"]
    fn real_system_vault_roundtrip_isolated_record() {
        let key = format!(
            "test:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = remove(&self.0);
            }
        }
        let _cleanup = Cleanup(key.clone());
        assert!(load(&key).unwrap().is_none());
        let state = LocalState {
            secrets: std::collections::BTreeMap::from([(
                "access_token".into(),
                "fake-vault-test-token".into(),
            )]),
            storage: serde_json::json!({"test":true}),
        };
        save(&key, &state).unwrap();
        let restored = load(&key).unwrap().unwrap();
        assert_eq!(restored.secrets, state.secrets);
        assert_eq!(restored.storage, state.storage);
        assert!(load(&format!("{key}-other-workspace")).unwrap().is_none());
        remove(&key).unwrap();
        assert!(load(&key).unwrap().is_none());
    }
}
