//! Client-owned provider discovery. No server installation or provider list is required.
use acheflow_extension_api::integration::{ExecutionSide, GuestManifest};
use std::collections::BTreeMap;
mod bundle {
    include!(concat!(env!("OUT_DIR"), "/client_extensions.rs"));
}

pub struct Installed {
    pub manifest: GuestManifest,
    pub module: Vec<u8>,
}
#[derive(Default)]
pub struct ClientCatalog {
    pub installed: BTreeMap<String, Installed>,
    pub errors: Vec<String>,
}
impl ClientCatalog {
    pub fn bundled() -> Self {
        let mut result = Self::default();
        for (manifest, module) in bundle::BUNDLED {
            result.insert(manifest.as_bytes(), module);
        }
        result
    }
    fn insert(&mut self, bytes: &[u8], module: &[u8]) {
        if let Err(error) = self.load(bytes, module) {
            self.errors.push(error);
        }
    }
    fn load(&mut self, bytes: &[u8], module: &[u8]) -> Result<(), String> {
        if bytes.len() > 65536 || module.len() > 4 * 1024 * 1024 {
            return Err("client installation exceeds size limit".into());
        }
        let manifest: GuestManifest =
            serde_json::from_slice(bytes).map_err(|_| "invalid client manifest")?;
        manifest.validate()?;
        if manifest.execution_side != ExecutionSide::Client {
            return Ok(());
        }
        if !module.starts_with(b"\0asm\x01\0\0\0") {
            return Err("invalid client WASM module".into());
        }
        if self.installed.len() >= 8 || self.installed.contains_key(&manifest.id) {
            return Err("duplicate client installation or catalog limit".into());
        }
        self.installed.insert(
            manifest.id.clone(),
            Installed {
                manifest,
                module: module.to_vec(),
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
        let mut entries: Vec<_> = entries.flatten().take(64).map(|e| e.path()).collect();
        entries.sort();
        for dir in entries {
            if !dir.join("manifest.json").is_file() {
                continue;
            }
            let load = || -> Result<(Vec<u8>, Vec<u8>), String> {
                if std::fs::metadata(dir.join("manifest.json"))
                    .map_err(|_| "client manifest unavailable")?
                    .len()
                    > 65536
                {
                    return Err("client manifest exceeds 64 KiB".into());
                }
                let bytes = std::fs::read(dir.join("manifest.json"))
                    .map_err(|_| "cannot read client manifest")?;
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
            };
            match load() {
                Ok((bytes, module)) if !bytes.is_empty() => result.insert(&bytes, &module),
                Ok(_) => {}
                Err(error) => result.errors.push(error),
            }
        }
        result
    }
}
