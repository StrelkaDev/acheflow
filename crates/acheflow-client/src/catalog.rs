//! Client-owned provider discovery. No server installation or provider list is required.
use acheflow_extension_api::integration::{ExecutionSide, GuestManifest};
use std::collections::BTreeMap;
mod bundle {
    include!(concat!(env!("OUT_DIR"), "/client_extensions.rs"));
}

pub struct Installed {
    pub manifest: GuestManifest,
    pub module: Vec<u8>,
    pub browser_auth: Option<String>,
}
#[derive(Default)]
pub struct ClientCatalog {
    pub installed: BTreeMap<String, Installed>,
    pub errors: Vec<String>,
    sources: BTreeMap<String, Vec<String>>,
}
impl ClientCatalog {
    pub fn bundled() -> Self {
        Self::from_bundled(bundle::BUNDLED)
    }
    fn from_bundled(installations: &[(&str, &[u8], Option<&str>)]) -> Self {
        let mut result = Self::default();
        for (index, (manifest, module, browser_auth)) in installations.iter().enumerate() {
            let source = format!("bundled installation #{}", index + 1);
            if result.insert(manifest.as_bytes(), module, &source)
                && let Ok(value) = serde_json::from_str::<GuestManifest>(manifest)
                && let Some(installed) = result.installed.get_mut(&value.id)
            {
                installed.browser_auth = browser_auth.map(str::to_owned);
            }
        }
        result
    }
    fn insert(&mut self, bytes: &[u8], module: &[u8], source: &str) -> bool {
        match self.load(bytes, module, source) {
            Ok(()) => true,
            Err(error) => {
                self.errors.push(format!("{source}: {error}"));
                false
            }
        }
    }
    fn load(&mut self, bytes: &[u8], module: &[u8], source: &str) -> Result<(), String> {
        if bytes.len() > 65536 || module.len() > 4 * 1024 * 1024 {
            return Err("client installation exceeds size limit".into());
        }
        let manifest: GuestManifest =
            serde_json::from_slice(bytes).map_err(|_| "invalid client manifest")?;
        manifest.validate()?;
        if manifest.execution_side != ExecutionSide::Client {
            return Err("installation does not execute on the client".into());
        }
        let sources = self.sources.entry(manifest.id.clone()).or_default();
        sources.push(source.into());
        if sources.len() > 1 {
            self.installed.remove(&manifest.id);
            return Err(format!(
                "conflicting extension ID '{}'; all installations disabled: {}",
                manifest.id,
                sources.join(", ")
            ));
        }
        if !module.starts_with(b"\0asm\x01\0\0\0") {
            return Err("invalid client WASM module".into());
        }
        if self.installed.len() >= 8 {
            return Err("client catalog limit reached".into());
        }
        self.installed.insert(
            manifest.id.clone(),
            Installed {
                manifest,
                module: module.to_vec(),
                browser_auth: None,
            },
        );
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    pub fn discover() -> Self {
        Self::bundled()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn discover() -> Self {
        if let Some(root) = std::env::var_os("ACHEFLOW_CLIENT_EXTENSIONS_DIR") {
            return Self::discover_at(std::path::Path::new(&root));
        }
        let local = std::path::Path::new("extensions");
        if local.is_dir() {
            return Self::discover_at(local);
        }
        if let Some(root) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("extensions")))
            .filter(|p| p.is_dir())
        {
            return Self::discover_at(&root);
        }
        Self::bundled()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn discover_at(root: &std::path::Path) -> Self {
        let mut result = Self::default();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => {
                result
                    .errors
                    .push("client extension directory is unavailable".into());
                return result;
            }
        };
        let Ok(mut entries) = entries
            .take(65)
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()
        else {
            result.errors.push(
                "client extension directory could not be fully read; catalog disabled".into(),
            );
            return result;
        };
        if entries.len() > 64 {
            result
                .errors
                .push("client extension directory exceeds scan limit; catalog disabled".into());
            return result;
        }
        entries.sort();
        for dir in entries {
            if !dir.join("manifest.json").is_file() {
                continue;
            }
            if let Err(error) = result.load_directory(&dir) {
                result.errors.push(format!("{}: {error}", dir.display()));
            }
        }
        result
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn load_directory(&mut self, dir: &std::path::Path) -> Result<(), String> {
        let (bytes, module) = read_installation(dir)?;
        if bytes.is_empty() || !self.insert(&bytes, &module, &dir.display().to_string()) {
            return Ok(());
        }
        let manifest: GuestManifest =
            serde_json::from_slice(&bytes).map_err(|_| "invalid client manifest")?;
        let installed = self
            .installed
            .get_mut(&manifest.id)
            .ok_or("client installation unavailable")?;
        installed.browser_auth = read_browser_authorization(dir, &manifest)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
fn read_installation(dir: &std::path::Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    if std::fs::metadata(dir.join("manifest.json"))
        .map_err(|_| "client manifest unavailable")?
        .len()
        > 65536
    {
        return Err("client manifest exceeds 64 KiB".into());
    }
    let bytes =
        std::fs::read(dir.join("manifest.json")).map_err(|_| "cannot read client manifest")?;
    if bytes.len() > 65536 {
        return Err("client manifest exceeds 64 KiB".into());
    }
    let manifest: GuestManifest =
        serde_json::from_slice(&bytes).map_err(|_| "invalid client manifest")?;
    manifest.validate()?;
    if manifest.execution_side != ExecutionSide::Client {
        return Ok((vec![], vec![]));
    }
    let directory = dir.canonicalize().map_err(|_| "invalid client directory")?;
    let path = dir
        .join(&manifest.module)
        .canonicalize()
        .map_err(|_| "client WASM module is missing")?;
    if !path.starts_with(&directory)
        || std::fs::metadata(&path)
            .map_err(|_| "client module unavailable")?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("client module is outside installation or too large".into());
    }
    Ok((
        bytes,
        std::fs::read(path).map_err(|_| "cannot read client module")?,
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_browser_authorization(
    dir: &std::path::Path,
    manifest: &GuestManifest,
) -> Result<Option<String>, String> {
    let Some(name) = manifest
        .authentication
        .as_ref()
        .and_then(|a| a["browser_module"].as_str())
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let source = dir.join(name);
    let valid = source
        .canonicalize()
        .ok()
        .zip(dir.canonicalize().ok())
        .is_some_and(|(file, root)| file.starts_with(root));
    if !valid || !std::fs::metadata(&source).is_ok_and(|m| m.len() <= 32768) {
        return Err("invalid browser authorization module".into());
    }
    let source =
        std::fs::read_to_string(source).map_err(|_| "cannot read browser authorization module")?;
    if source.len() > 32768 {
        return Err("browser authorization module exceeds 32 KiB".into());
    }
    Ok(Some(source))
}
