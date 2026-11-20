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

#[cfg(target_os = "macos")]
pub(crate) fn migrate(old: &str, new: &str) -> Result<(), String> {
    native::with_store(|b| store::migrate(b, old, new))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn migrate(_: &str, _: &str) -> Result<(), String> {
    Ok(())
}

// Device-to-server login has a separate lifetime and a single OS record. Provider
// guests never receive this namespace or its contents.
#[cfg(target_os = "macos")]
pub(crate) fn device_session(
    endpoint: &str,
    update: Option<Option<&str>>,
) -> Result<Option<String>, String> {
    use store::Backend;
    const SERVICE: &str = "Acheflow server access v1";
    const RECORD: &str = "device-sessions-v2";
    native::with_store(|backend| {
        let bytes = backend.read(SERVICE, RECORD)?;
        if bytes.as_ref().is_some_and(|b| b.len() > 64 * 1024) {
            return Err("Device session container exceeds limit.".into());
        }
        let mut sessions: std::collections::BTreeMap<String, String> = match bytes {
            Some(b) => {
                serde_json::from_slice(&b).map_err(|_| "Invalid device session container.")?
            }
            None => Default::default(),
        };
        let legacy = backend.read(SERVICE, endpoint)?;
        let mut changed = false;
        if !sessions.contains_key(endpoint)
            && let Some(bytes) = legacy.as_ref()
        {
            let token =
                String::from_utf8(bytes.clone()).map_err(|_| "Invalid device session token.")?;
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid device session token.".into());
            }
            sessions.insert(endpoint.into(), token);
            changed = true;
        }
        if let Some(update) = update {
            match update {
                Some(token) => {
                    sessions.insert(endpoint.into(), token.into());
                }
                None => {
                    sessions.remove(endpoint);
                }
            }
            changed = true;
        }
        if sessions.len() > 64 {
            return Err("Device session container is full.".into());
        }
        if changed {
            if sessions.is_empty() {
                backend.delete(SERVICE, RECORD)?;
            } else {
                backend.write(
                    SERVICE,
                    RECORD,
                    &serde_json::to_vec(&sessions).map_err(|_| "Cannot save device sessions.")?,
                )?;
            }
        }
        if legacy.is_some() {
            backend.delete(SERVICE, endpoint)?;
        }
        Ok(sessions.remove(endpoint))
    })
}

/// A display label only: renaming a workspace never changes its credential identity.
pub(crate) fn name_workspace(
    namespace: &str,
    workspace: &str,
    name: &str,
    role: &str,
) -> Result<(), String> {
    let label = workspace_label(namespace, workspace, name, role)?;
    #[cfg(target_os = "macos")]
    {
        native::name_workspace(
            &serde_json::to_string(&(namespace, workspace))
                .map_err(|_| "Invalid workspace identity.")?,
            &label,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (namespace, workspace, label);
        Ok(())
    }
}

fn workspace_label(
    namespace: &str,
    workspace: &str,
    name: &str,
    role: &str,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let server = namespace
        .strip_prefix("client:")
        .or_else(|| namespace.strip_prefix("server:"))
        .unwrap_or(namespace);
    let identity =
        serde_json::to_vec(&(server, workspace)).map_err(|_| "Invalid workspace identity.")?;
    let digest = format!("{:x}", Sha256::digest(&identity));
    let display = name
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect::<String>();
    let display = if display.trim().is_empty() {
        "Workspace"
    } else {
        display.trim()
    };
    Ok(format!("Acheflow ({display} · {}) {role}", &digest[..8]))
}

#[cfg(test)]
mod label_tests {
    use super::*;
    #[test]
    fn workspace_suffix_matches_both_hosts_and_survives_renaming() {
        let client =
            workspace_label("client:server-id", "workspace-id", "Studio", "Client").unwrap();
        let server =
            workspace_label("server:server-id", "workspace-id", "Studio", "Server").unwrap();
        assert_eq!(
            client.trim_end_matches("Client"),
            server.trim_end_matches("Server")
        );
        let renamed =
            workspace_label("client:server-id", "workspace-id", "Renamed", "Client").unwrap();
        assert_eq!(client.split(" · ").nth(1), renamed.split(" · ").nth(1));
        for (namespace, workspace) in [
            ("client:another-server", "workspace-id"),
            ("client:server-id", "another-workspace"),
        ] {
            assert_ne!(
                client,
                workspace_label(namespace, workspace, "Studio", "Client").unwrap()
            );
        }
        assert!(
            !workspace_label("client:server-id", "workspace-id", "\nStudio\r", "Client")
                .unwrap()
                .contains('\n')
        );
    }
}
