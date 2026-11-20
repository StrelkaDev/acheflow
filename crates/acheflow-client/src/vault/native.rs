use super::store::Backend;
use std::{
    fs::{File, OpenOptions, TryLockError},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant},
};

// Cache contains only process memory. The marker carries no credentials and is
// changed under the shared lock before every OS-vault mutation.
type Records = std::collections::BTreeMap<(String, String), Option<Vec<u8>>>;
#[derive(Default)]
struct Cache {
    revision: Vec<u8>,
    records: Records,
    labels: std::collections::BTreeMap<String, String>,
}
fn cache() -> &'static std::sync::Mutex<Cache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub(super) struct System {
    marker: std::path::PathBuf,
}
impl System {
    fn refresh(&self) -> Result<(), String> {
        let revision = match std::fs::read(&self.marker) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err("Credential cache revision is unavailable.".into()),
        };
        let mut cache = cache()
            .lock()
            .map_err(|_| "Credential cache unavailable.")?;
        if cache.revision != revision {
            cache.records.clear();
            cache.revision = revision;
        }
        Ok(())
    }
    fn invalidate(&self) -> Result<(), String> {
        use std::io::Write;
        let mut revision = [0u8; 16];
        getrandom::getrandom(&mut revision)
            .map_err(|_| "Credential cache revision is unavailable.")?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&self.marker)
            .map_err(|_| "Credential cache revision is unavailable.")?;
        file.write_all(&revision)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Credential cache revision is unavailable.")?;
        let mut cache = cache()
            .lock()
            .map_err(|_| "Credential cache unavailable.")?;
        cache.records.clear();
        cache.revision = revision.to_vec();
        Ok(())
    }
    fn label_workspace(&self, key: &str, label: &str) -> Result<(), String> {
        use security_framework::item::{
            ItemClass, ItemSearchOptions, ItemUpdateOptions, update_item,
        };
        let mut matching = ItemSearchOptions::new();
        matching
            .class(ItemClass::generic_password())
            .service(super::store::WORKSPACE_SERVICE)
            .account(key)
            .label(label)
            .load_refs(true);
        match matching.search() {
            Ok(_) => return Ok(()),
            Err(e) if e.code() == -25300 => {}
            Err(_) => return Err("Could not read the workspace credential label.".into()),
        }
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::generic_password())
            .service(super::store::WORKSPACE_SERVICE)
            .account(key);
        let mut update = ItemUpdateOptions::new();
        update.set_label(label);
        match update_item(&search, &update) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(_) => Err("Could not name the workspace credential container.".into()),
        }
    }
    fn remember(&self, service: &str, key: &str, value: Option<Vec<u8>>) {
        if value.as_ref().is_some_and(|v| v.len() > 4 * 1024 * 1024) {
            return;
        }
        if let Ok(mut cache) = cache().lock() {
            if cache.records.len() >= 64 {
                cache.records.clear();
            }
            cache.records.insert((service.into(), key.into()), value);
        }
    }
}
impl Backend for System {
    fn read(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        self.refresh()?;
        if let Some(value) = cache()
            .lock()
            .map_err(|_| "Credential cache unavailable.")?
            .records
            .get(&(service.into(), key.into()))
            .cloned()
        {
            return Ok(value);
        }
        let value = match security_framework::passwords::get_generic_password(service, key) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.code() == -25300 => None,
            Err(_) => return Err("System credential storage could not be opened.".into()),
        };
        self.remember(service, key, value.clone());
        Ok(value)
    }
    fn write(&self, service: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.invalidate()?;
        security_framework::passwords::set_generic_password(service, key, bytes)
            .map_err(|_| "System credential storage could not save this account.")?;
        self.remember(service, key, Some(bytes.to_vec()));
        if service == super::store::WORKSPACE_SERVICE {
            let label = cache()
                .lock()
                .ok()
                .and_then(|cache| cache.labels.get(key).cloned());
            if let Some(label) = label {
                self.label_workspace(key, &label)?;
            }
        }
        Ok(())
    }
    fn delete(&self, service: &str, key: &str) -> Result<(), String> {
        self.refresh()?;
        if cache()
            .lock()
            .map_err(|_| "Credential cache unavailable.")?
            .records
            .get(&(service.into(), key.into()))
            == Some(&None)
        {
            return Ok(());
        }
        self.invalidate()?;
        let result = match security_framework::passwords::delete_generic_password(service, key) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err("System credential storage could not remove this account.".into()),
        };
        if result.is_ok() {
            self.remember(service, key, None);
        }
        result
    }
}

pub(super) fn with_store<T>(
    operation: impl FnOnce(&System) -> Result<T, String>,
) -> Result<T, String> {
    static OPERATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Credential storage unavailable.")?;
    let home = std::env::var_os("HOME").ok_or("Home directory is unavailable.")?;
    let directory = std::path::PathBuf::from(home).join(".acheflow");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|_| "Could not open credential storage lock directory.")?;
    let _lock = lock(&directory.join("workspace-vault.lock"))?;
    operation(&System {
        marker: directory.join("workspace-vault.revision"),
    })
}

// A process-local mutex would let two desktop processes overwrite other accounts.
// This file contains no credentials; the kernel releases its lock on process exit.
pub(super) fn lock(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "Could not open credential storage lock.")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return Err("Credential storage is busy or unavailable. Try again.".into()),
        }
    }
}

pub(super) fn name_workspace(key: &str, label: &str) -> Result<(), String> {
    if cache()
        .lock()
        .map_err(|_| "Credential cache unavailable.")?
        .labels
        .get(key)
        .is_some_and(|old| old == label)
    {
        return Ok(());
    }
    with_store(|system| {
        system.label_workspace(key, label)?;
        cache()
            .lock()
            .map_err(|_| "Credential cache unavailable.")?
            .labels
            .insert(key.into(), label.into());
        Ok(())
    })
}
