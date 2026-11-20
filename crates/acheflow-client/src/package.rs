//! Validate in memory, then publish immutable host-specific package versions.
//! No ZIP entry is ever used as an extraction path. No guest action is executed.
use acheflow_control::extensions::Guest;
use acheflow_extension_api::{
    integration::{ExecutionSide, GuestManifest},
    package::PackageReview,
    parts::{ExecutionHost, PartKind},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const MAX_ARCHIVE_BYTES: usize = 8 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 12 * 1024 * 1024;
const STAGE_MS: u64 = 60 * 60 * 1000;
#[derive(Clone)]
pub struct Prepared {
    review: PackageReview,
    manifest: GuestManifest,
    module: Vec<u8>,
    browser_auth: Option<String>,
    files: BTreeMap<String, Vec<u8>>,
}
impl Prepared {
    pub fn review(&self) -> &PackageReview {
        &self.review
    }
    pub fn manifest(&self) -> &GuestManifest {
        &self.manifest
    }
    pub fn module(&self) -> &[u8] {
        &self.module
    }
    pub fn into_installed(self) -> crate::catalog::Installed {
        crate::catalog::Installed {
            manifest: self.manifest,
            module: self.module,
            browser_auth: self.browser_auth,
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}
pub fn inspect(bytes: &[u8], host: ExecutionHost) -> Result<Prepared, String> {
    if bytes.is_empty() || bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("Package exceeds the 8 MiB archive limit.".into());
    }
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Invalid ZIP package.")?;
    if archive.is_empty() || archive.len() > 32 {
        return Err("Package must contain at most 32 entries.".into());
    }
    let mut entries = BTreeMap::new();
    let mut expanded = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| "Invalid ZIP entry.")?;
        let name = entry
            .name()
            .map_err(|_| "Invalid ZIP filename.")?
            .into_owned();
        if entry.is_symlink()
            || entry.encrypted()
            || entry.enclosed_name().is_none()
            || name.contains('\\')
            || name.split('/').any(|p| p == ".." || p == ".")
        {
            return Err("Package contains an unsafe, linked or encrypted entry.".into());
        }
        if entry.is_dir() {
            continue;
        }
        if !entry.is_file()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
        {
            return Err("Package entries must be regular files.".into());
        }
        let limit = if name.ends_with("manifest.json") {
            65536
        } else {
            4 * 1024 * 1024
        };
        if entry.size() > limit as u64 {
            return Err("Package entry exceeds its size limit.".into());
        }
        let mut data = Vec::new();
        entry
            .take(limit as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|_| "Could not read package entry.")?;
        expanded = expanded
            .checked_add(data.len())
            .ok_or("Package size overflow.")?;
        if data.len() > limit || expanded > MAX_EXPANDED_BYTES {
            return Err("Expanded package exceeds limit.".into());
        }
        if entries.insert(name, data).is_some() {
            return Err("Duplicate package entry.".into());
        }
    }
    let manifests: Vec<_> = entries
        .keys()
        .filter(|name| name.as_str() == "manifest.json" || name.ends_with("/manifest.json"))
        .cloned()
        .collect();
    if manifests.len() != 1 {
        return Err("Package must contain exactly one manifest.json.".into());
    }
    let manifest_path = &manifests[0];
    let prefix = manifest_path.strip_suffix("manifest.json").unwrap();
    if !prefix.is_empty() && (!prefix.ends_with('/') || !safe_name(prefix.trim_end_matches('/'))) {
        return Err("Package must be flat or contain one extension directory.".into());
    }
    let package: GuestManifest =
        serde_json::from_slice(&entries[manifest_path]).map_err(|_| "Invalid package manifest.")?;
    package.validate()?;
    if package.parts.is_none() {
        return Err(
            "Installable packages require parts_version 1; rebuild the legacy package.".into(),
        );
    }
    let mut declared = BTreeSet::from(["manifest.json".to_owned()]);
    for target in [ExecutionHost::Client, ExecutionHost::Server] {
        if let Some(part) = package.select_part(target)? {
            if !safe_name(&part.module) || !part.module.ends_with(".wasm") {
                return Err("WASM artifacts require a local .wasm filename.".into());
            }
            declared.insert(part.module.clone());
            if let Some(name) = part
                .authentication
                .as_ref()
                .and_then(|auth| auth["browser_module"].as_str())
            {
                if !safe_name(name) || !name.ends_with(".js") {
                    return Err(
                        "Browser authorization artifacts require a local .js filename.".into(),
                    );
                }
                declared.insert(name.to_owned());
            }
        }
    }
    if entries.keys().any(|path| {
        path.strip_prefix(prefix)
            .is_none_or(|name| !safe_name(name) || !declared.contains(name))
    }) {
        return Err("Package contains undeclared artifacts or nested paths.".into());
    }
    let manifest = package
        .select_part(host)?
        .ok_or("Package does not declare the selected host part.")?;
    let module = entries
        .get(&format!("{prefix}{}", manifest.module))
        .ok_or("Selected WASM artifact is missing.")?
        .clone();
    Guest::load_entry_point(
        &module,
        manifest.capabilities.clone(),
        &manifest.entry_point,
    )
    .map_err(|_| "Selected WASM module or entry point is invalid.")?;
    let browser_name = manifest
        .authentication
        .as_ref()
        .and_then(|auth| auth["browser_module"].as_str());
    let browser_auth = if host == ExecutionHost::Client {
        browser_name
            .map(|name| {
                let bytes = entries
                    .get(&format!("{prefix}{name}"))
                    .ok_or("Browser authorization artifact is missing.")?;
                if bytes.len() > 32768 {
                    return Err("Browser authorization module exceeds limit.");
                }
                String::from_utf8(bytes.clone())
                    .map_err(|_| "Browser authorization module is not UTF-8.")
            })
            .transpose()?
    } else {
        None
    };
    let mut files = BTreeMap::from([(manifest.module.clone(), module.clone())]);
    if let (Some(name), Some(source)) = (browser_name, &browser_auth) {
        files.insert(name.into(), source.as_bytes().to_vec());
    }
    let mut disk_package = package.clone();
    let parts = disk_package
        .parts
        .as_mut()
        .ok_or("Package parts are missing.")?;
    match host {
        ExecutionHost::Client => parts.server = None,
        ExecutionHost::Server => parts.client = None,
    }
    files.insert(
        "manifest.json".into(),
        serde_json::to_vec(&disk_package).map_err(|_| "Could not encode selected package.")?,
    );
    let review = PackageReview {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        digest: digest(bytes),
        host,
        kind: if manifest.execution_side == ExecutionSide::Server {
            PartKind::Logic
        } else {
            PartKind::Provider
        },
        capabilities: manifest.capabilities.clone(),
        supported_actions: manifest.supported_actions.clone().unwrap_or_default(),
        services: manifest.services.clone(),
        settings: manifest.settings.clone(),
        requires_authorization: manifest.authentication.is_some()
            || !manifest.secret_fields.is_empty(),
        replaces: None,
        execution_allowed: false,
    };
    Ok(Prepared {
        review,
        manifest,
        module,
        browser_auth,
        files,
    })
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Active {
    digest: String,
    execution_allowed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    active: BTreeMap<String, Active>,
}
pub struct Store {
    root: PathBuf,
    host: ExecutionHost,
    state: Mutex<State>,
    _lock: File,
}
impl Store {
    pub fn open(root: PathBuf, host: ExecutionHost) -> Result<Self, String> {
        private_directory(&root)?;
        let lock = private_file(&root.join("store.lock"), false)?;
        lock.try_lock()
            .map_err(|_| "Another process is updating this package store.")?;
        for name in ["staging", "versions"] {
            private_directory(&root.join(name))?;
        }
        let path = root.join("active.json");
        let state = if path.exists() {
            let bytes = read_regular(&path, 65536)?;
            serde_json::from_slice::<State>(&bytes).map_err(|_| "Invalid package store.")?
        } else {
            State {
                version: 1,
                active: BTreeMap::new(),
            }
        };
        if state.version != 1
            || state.active.len() > 8
            || state.active.iter().any(|(id, active)| {
                !acheflow_extension_api::integration::valid_id(id, 64)
                    || !valid_digest(&active.digest)
            })
        {
            return Err("Invalid active package index.".into());
        }
        Ok(Self {
            root,
            host,
            state: Mutex::new(state),
            _lock: lock,
        })
    }
    pub fn stage(&self, bytes: &[u8], now: u64) -> Result<PackageReview, String> {
        self.stage_checked(bytes, now, || Ok(()))
    }
    pub fn stage_checked(
        &self,
        bytes: &[u8],
        now: u64,
        authorize: impl FnOnce() -> Result<(), String>,
    ) -> Result<PackageReview, String> {
        let prepared = inspect(bytes, self.host)?;
        authorize()?;
        let _state = self.state.lock().unwrap();
        let mut count = 0;
        for entry in std::fs::read_dir(self.root.join("staging"))
            .map_err(|_| "Cannot open package staging.")?
        {
            let path = entry.map_err(|_| "Cannot read staged packages.")?.path();
            if path.extension().and_then(|v| v.to_str()) != Some("zip") {
                continue;
            }
            if std::fs::symlink_metadata(&path)
                .map_err(|_| "Invalid staged package.")?
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|t| now.saturating_sub(t.as_millis() as u64) > STAGE_MS)
            {
                std::fs::remove_file(path).map_err(|_| "Cannot expire staged package.")?;
            } else {
                count += 1;
            }
        }
        if count >= 16
            && !self
                .root
                .join("staging")
                .join(format!("{}.zip", prepared.review.digest))
                .exists()
        {
            return Err("Package staging is full; wait for expiry.".into());
        }
        atomic_write(
            &self
                .root
                .join("staging")
                .join(format!("{}.zip", prepared.review.digest)),
            bytes,
        )?;
        let mut review = prepared.review;
        review.replaces = _state.active.get(&review.id).map(|p| p.digest.clone());
        Ok(review)
    }
    pub fn prepare(&self, hash: &str, expected: Option<&str>) -> Result<Prepared, String> {
        if !valid_digest(hash) {
            return Err("Invalid package digest.".into());
        }
        let path = self.root.join("staging").join(format!("{hash}.zip"));
        if std::fs::symlink_metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|elapsed| elapsed.as_millis() > u128::from(STAGE_MS))
        {
            return Err("Staged package expired; upload it again.".into());
        }
        let bytes = read_regular(&path, MAX_ARCHIVE_BYTES)?;
        let mut prepared = inspect(&bytes, self.host)?;
        if prepared.review.digest != hash {
            return Err("Staged package digest mismatch.".into());
        }
        let state = self.state.lock().unwrap();
        let current = state
            .active
            .get(&prepared.review.id)
            .map(|entry| entry.digest.as_str());
        if current != expected {
            return Err("Installed package changed; review it again.".into());
        }
        if current.is_none() && state.active.len() >= 8 {
            return Err("Installed package limit reached.".into());
        }
        prepared.review.replaces = current.map(str::to_owned);
        Ok(prepared)
    }
    pub fn commit(&self, prepared: &Prepared) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state
            .active
            .get(&prepared.review.id)
            .map(|p| p.digest.as_str())
            != prepared.review.replaces.as_deref()
        {
            return Err("Installed package changed; review it again.".into());
        }
        let version = self.root.join("versions").join(&prepared.review.digest);
        private_directory(&version)?;
        for (name, bytes) in &prepared.files {
            let path = version.join(name);
            if path.exists() {
                if read_regular(&path, 4 * 1024 * 1024)? != *bytes {
                    return Err("Existing package version is damaged.".into());
                }
            } else {
                let mut file = private_file(&path, true)?;
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Cannot persist package artifacts.")?;
            }
        }
        let mut active = state.active.clone();
        active.insert(
            prepared.review.id.clone(),
            Active {
                digest: prepared.review.digest.clone(),
                execution_allowed: false,
            },
        );
        let next = State { version: 1, active };
        atomic_write(
            &self.root.join("active.json"),
            &serde_json::to_vec(&next).map_err(|_| "Cannot encode installed packages.")?,
        )?;
        *state = next;
        for entry in std::fs::read_dir(self.root.join("versions"))
            .into_iter()
            .flatten()
        {
            let Ok(entry) = entry else {
                continue;
            };
            let name = entry.file_name();
            let Some(hash) = name.to_str() else {
                continue;
            };
            if valid_digest(hash)
                && !state.active.values().any(|active| active.digest == hash)
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let _ = std::fs::remove_file(
            self.root
                .join("staging")
                .join(format!("{}.zip", prepared.review.digest)),
        );
        Ok(())
    }
    pub fn id_for_digest(&self, hash: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .active
            .iter()
            .find(|(_, entry)| entry.digest == hash)
            .map(|(id, _)| id.clone())
    }
    pub fn execution_allowed(&self, id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .active
            .get(id)
            .is_none_or(|p| p.execution_allowed)
    }
    pub fn allow_execution(&self, id: &str, hash: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let mut active = state.active.clone();
        let entry = active
            .get_mut(id)
            .filter(|p| p.digest == hash)
            .ok_or("Installed package changed; review it again.")?;
        entry.execution_allowed = true;
        let next = State { version: 1, active };
        atomic_write(
            &self.root.join("active.json"),
            &serde_json::to_vec(&next).map_err(|_| "Cannot encode installed packages.")?,
        )?;
        *state = next;
        Ok(())
    }
    pub fn installed(&self) -> Result<Vec<Prepared>, String> {
        let state = self.state.lock().unwrap();
        let mut result = Vec::new();
        for (id, entry) in &state.active {
            let directory = self.root.join("versions").join(&entry.digest);
            let bytes = read_regular(&directory.join("manifest.json"), 65536)?;
            let package: GuestManifest =
                serde_json::from_slice(&bytes).map_err(|_| "Installed manifest is invalid.")?;
            let manifest = package
                .select_part(self.host)?
                .ok_or("Installed package part is missing.")?;
            if &manifest.id != id {
                return Err("Installed package identity changed.".into());
            }
            let module = read_regular(&directory.join(&manifest.module), 4 * 1024 * 1024)?;
            Guest::load_entry_point(
                &module,
                manifest.capabilities.clone(),
                &manifest.entry_point,
            )
            .map_err(|_| "Installed WASM module is invalid.")?;
            let browser_auth = if self.host == ExecutionHost::Client {
                manifest
                    .authentication
                    .as_ref()
                    .and_then(|a| a["browser_module"].as_str())
                    .map(|name| {
                        read_regular(&directory.join(name), 32768).and_then(|b| {
                            String::from_utf8(b)
                                .map_err(|_| "Invalid browser authorization module.".into())
                        })
                    })
                    .transpose()?
            } else {
                None
            };
            let review = PackageReview {
                id: id.clone(),
                name: manifest.name.clone(),
                digest: entry.digest.clone(),
                host: self.host,
                kind: if manifest.execution_side == ExecutionSide::Server {
                    PartKind::Logic
                } else {
                    PartKind::Provider
                },
                capabilities: manifest.capabilities.clone(),
                supported_actions: manifest.supported_actions.clone().unwrap_or_default(),
                services: manifest.services.clone(),
                settings: manifest.settings.clone(),
                requires_authorization: manifest.authentication.is_some()
                    || !manifest.secret_fields.is_empty(),
                replaces: None,
                execution_allowed: entry.execution_allowed,
            };
            result.push(Prepared {
                review,
                manifest,
                module,
                browser_auth,
                files: BTreeMap::new(),
            });
        }
        Ok(result)
    }
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn private_directory(path: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| "Cannot create private package directory.")?;
    if !std::fs::symlink_metadata(path)
        .map_err(|_| "Cannot inspect package directory.")?
        .is_dir()
    {
        return Err("Package directory cannot be a link.".into());
    }
    Ok(())
}
fn private_file(path: &Path, new: bool) -> Result<File, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err("Package file cannot be a link.".into()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("Cannot inspect package file.".into());
        }
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if new {
        options.create_new(true);
    } else {
        options.create(true).truncate(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "Cannot open private package file.".into())
}
fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "Package artifact is unavailable.")?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err("Invalid package artifact.".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "Cannot open package artifact.")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read package artifact.")?;
    if bytes.len() > limit {
        return Err("Package artifact exceeds limit.".into());
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| "Cannot create package transaction.")?;
    let temporary = path.with_extension(format!("{}.tmp", digest(&random)));
    let result = (|| {
        let mut file = private_file(&temporary, true)?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Cannot save package transaction.")?;
        std::fs::rename(&temporary, path)
            .map_err(|_| "Cannot publish package transaction.".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
pub fn client_root() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("HOME").ok_or("Home directory is unavailable.")?)
            .join(".acheflow/packages/client"),
    )
}
#[cfg(test)]
mod tests;
