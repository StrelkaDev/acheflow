//! Workspace containers over an OS vault. Callers hold the cross-process lock.
use crate::local::LocalState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const LEGACY_SERVICE: &str = "Acheflow client extension storage v1";
pub(super) const WORKSPACE_SERVICE: &str = "Acheflow workspace credentials v1";
const MAX_ACCOUNT_BYTES: usize = 256 * 1024;
const MAX_WORKSPACE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ACCOUNTS: usize = 64;

pub(super) trait Backend {
    fn read(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&self, service: &str, key: &str, bytes: &[u8]) -> Result<(), String>;
    fn delete(&self, service: &str, key: &str) -> Result<(), String>;
}

// No Debug: these structs contain raw credentials and must never enter diagnostics.
#[derive(PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    secrets: BTreeMap<String, String>,
    #[serde(default)]
    storage: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceVault {
    version: u32,
    accounts: BTreeMap<String, Account>,
    #[serde(default)]
    retired_legacy: BTreeSet<String>,
}

struct Identity {
    workspace: String,
    account: String,
}
impl Identity {
    fn parse(key: &str) -> Result<Self, String> {
        if key.len() > 64 * 1024 {
            return Err("Invalid local storage identity.".into());
        }
        let (server, workspace, extension, application): (String, String, String, String) =
            serde_json::from_str(key).map_err(|_| "Invalid local storage identity.")?;
        Ok(Self {
            workspace: serde_json::to_string(&(server, workspace))
                .map_err(|_| "Invalid workspace storage identity.")?,
            account: serde_json::to_string(&(extension, application))
                .map_err(|_| "Invalid account storage identity.")?,
        })
    }
}

fn decode_account(bytes: &[u8]) -> Result<Account, String> {
    if bytes.len() > MAX_ACCOUNT_BYTES {
        return Err("Stored extension data exceeds limit.".into());
    }
    serde_json::from_slice(bytes).map_err(|_| "Invalid local credential record.".into())
}

fn read_workspace(backend: &impl Backend, key: &str) -> Result<WorkspaceVault, String> {
    let Some(bytes) = backend.read(WORKSPACE_SERVICE, key)? else {
        return Ok(WorkspaceVault {
            version: 1,
            accounts: BTreeMap::new(),
            retired_legacy: BTreeSet::new(),
        });
    };
    if bytes.len() > MAX_WORKSPACE_BYTES {
        return Err("Stored workspace credentials exceed limit.".into());
    }
    let vault: WorkspaceVault =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid workspace credential record.")?;
    if vault.version != 1
        || vault.accounts.len() > MAX_ACCOUNTS
        || vault.retired_legacy.len() > MAX_ACCOUNTS
    {
        return Err("Invalid workspace credential record.".into());
    }
    for (key, account) in &vault.accounts {
        serde_json::from_str::<(String, String)>(key)
            .map_err(|_| "Invalid account storage identity.")?;
        encode_account(account)?;
    }
    Ok(vault)
}

fn encode_account(account: &Account) -> Result<Vec<u8>, String> {
    let bytes =
        serde_json::to_vec(account).map_err(|_| "Could not encode local credential record.")?;
    if bytes.len() > MAX_ACCOUNT_BYTES {
        return Err("Extension credential record exceeds limit.".into());
    }
    Ok(bytes)
}

fn write_workspace(
    backend: &impl Backend,
    key: &str,
    vault: &WorkspaceVault,
) -> Result<(), String> {
    if vault.accounts.is_empty() {
        return backend.delete(WORKSPACE_SERVICE, key);
    }
    if vault.accounts.len() > MAX_ACCOUNTS {
        return Err("Workspace account limit reached.".into());
    }
    let bytes =
        serde_json::to_vec(vault).map_err(|_| "Could not encode workspace credential record.")?;
    if bytes.len() > MAX_WORKSPACE_BYTES {
        return Err("Workspace credentials exceed limit.".into());
    }
    backend.write(WORKSPACE_SERVICE, key, &bytes)
}

pub(super) fn load(backend: &impl Backend, key: &str) -> Result<Option<LocalState>, String> {
    let identity = Identity::parse(key)?;
    let mut vault = read_workspace(backend, &identity.workspace)?;
    if !vault.accounts.contains_key(&identity.account) {
        let Some(bytes) = backend.read(LEGACY_SERVICE, key)? else {
            return Ok(None);
        };
        vault
            .accounts
            .insert(identity.account.clone(), decode_account(&bytes)?);
        // Never retire the only recoverable copy before the new write succeeds.
        write_workspace(backend, &identity.workspace, &vault)?;
    }
    retire_legacy(backend, key, &identity, &mut vault)?;
    let account = vault.accounts.remove(&identity.account).unwrap();
    Ok(Some(LocalState {
        secrets: account.secrets,
        storage: account.storage,
    }))
}

fn retire_legacy(
    backend: &impl Backend,
    key: &str,
    identity: &Identity,
    vault: &mut WorkspaceVault,
) -> Result<(), String> {
    if vault.retired_legacy.contains(&identity.account) {
        return Ok(());
    }
    if backend.read(LEGACY_SERVICE, key)?.is_some() {
        backend.delete(LEGACY_SERVICE, key)?;
    }
    // Persist completion only after deletion succeeds. Later reads of this slot
    // must not reopen the old per-extension key, including after process restart.
    vault.retired_legacy.insert(identity.account.clone());
    write_workspace(backend, &identity.workspace, vault)
}

pub(super) fn save(backend: &impl Backend, key: &str, state: &LocalState) -> Result<(), String> {
    if state.secrets.is_empty() {
        return remove(backend, key);
    }
    let identity = Identity::parse(key)?;
    let account = Account {
        secrets: state.secrets.clone(),
        storage: state.storage.clone(),
    };
    encode_account(&account)?;
    let mut vault = read_workspace(backend, &identity.workspace)?;
    if vault.accounts.get(&identity.account) != Some(&account) {
        vault.accounts.insert(identity.account.clone(), account);
        write_workspace(backend, &identity.workspace, &vault)?;
    }
    retire_legacy(backend, key, &identity, &mut vault)
}

pub(super) fn remove(backend: &impl Backend, key: &str) -> Result<(), String> {
    let identity = Identity::parse(key)?;
    let mut vault = read_workspace(backend, &identity.workspace)?;
    // A failed container update must not let an old record resurrect this account.
    backend.delete(LEGACY_SERVICE, key)?;
    vault.retired_legacy.remove(&identity.account);
    if vault.accounts.remove(&identity.account).is_some() {
        write_workspace(backend, &identity.workspace, &vault)?;
    }
    Ok(())
}

/// Move only this slot after a recoverable destination exists. Conflicting old
/// accounts remain available for explicit recovery; neighbors are untouched.
pub(super) fn migrate(backend: &impl Backend, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let source = load(backend, old)?;
    let target = load(backend, new)?;
    if let Some(source) = source {
        if let Some(target) = target {
            if source.secrets == target.secrets && source.storage == target.storage {
                remove(backend, old)?;
            }
        } else {
            save(backend, new, &source)?;
            remove(backend, old)?;
        }
    }
    Ok(())
}
