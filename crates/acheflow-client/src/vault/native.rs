use super::store::Backend;
use std::{
    fs::{File, OpenOptions, TryLockError},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant},
};

pub(super) struct System;
impl Backend for System {
    fn read(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        match security_framework::passwords::get_generic_password(service, key) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err("System credential storage could not be opened.".into()),
        }
    }
    fn write(&self, service: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
        security_framework::passwords::set_generic_password(service, key, bytes)
            .map_err(|_| "System credential storage could not save this account.".into())
    }
    fn delete(&self, service: &str, key: &str) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(service, key) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err("System credential storage could not remove this account.".into()),
        }
    }
}

pub(super) fn with_store<T>(
    operation: impl FnOnce(&System) -> Result<T, String>,
) -> Result<T, String> {
    let home = std::env::var_os("HOME").ok_or("Home directory is unavailable.")?;
    let directory = std::path::PathBuf::from(home).join(".acheflow");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|_| "Could not open credential storage lock directory.")?;
    let _lock = lock(&directory.join("workspace-vault.lock"))?;
    operation(&System)
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
