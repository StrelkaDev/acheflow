//! Read installation artifacts on the host; guests have no filesystem capability.
use acheflow_control::extensions::{Guest, GuestManifest};
use std::{collections::BTreeMap, path::Path};

pub struct Installed {
    pub manifest: GuestManifest,
    pub module: Vec<u8>,
}
pub fn discover() -> BTreeMap<String, Installed> {
    let mut installed = BTreeMap::new();
    for (manifest, module) in [
        (
            include_str!("../../../extensions/twitch/manifest.json"),
            include_bytes!("../../../extensions/twitch/twitch.wat").as_slice(),
        ),
        (
            include_str!("../../../extensions/sample/manifest.json"),
            include_bytes!("../../../extensions/sample/sample.wat").as_slice(),
        ),
    ] {
        let manifest: GuestManifest = serde_json::from_str(manifest).expect("bundled manifest");
        installed.insert(
            manifest.id.clone(),
            Installed {
                manifest,
                module: module.to_vec(),
            },
        );
    }
    let root = std::env::var("ACHEFLOW_EXTENSIONS_DIR").unwrap_or_else(|_| "extensions".into());
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten().take(32) {
            match load(&entry.path()) {
                Ok(extension)
                    if !installed.contains_key(&extension.manifest.id) && installed.len() < 8 =>
                {
                    installed.insert(extension.manifest.id.clone(), extension);
                }
                Ok(_) => {}
                Err(error) => eprintln!("Extension installation skipped: {error}"),
            }
        }
    }
    installed
}
fn load(directory: &Path) -> Result<Installed, String> {
    let directory = directory
        .canonicalize()
        .map_err(|_| "invalid extension directory")?;
    let manifest_path = directory.join("manifest.json");
    if std::fs::metadata(&manifest_path)
        .map_err(|_| "missing extension manifest")?
        .len()
        > 65536
    {
        return Err("manifest exceeds 64 KiB".into());
    }
    let manifest: GuestManifest = serde_json::from_slice(
        &std::fs::read(manifest_path).map_err(|_| "could not read manifest")?,
    )
    .map_err(|_| "invalid manifest JSON")?;
    manifest.validate()?;
    let module_path = directory
        .join(&manifest.module)
        .canonicalize()
        .map_err(|_| "extension module missing")?;
    if !module_path.starts_with(&directory)
        || std::fs::metadata(&module_path)
            .map_err(|_| "module unavailable")?
            .len()
            > 4 * 1024 * 1024
    {
        return Err("extension module is outside its directory or too large".into());
    }
    let module = std::fs::read(module_path).map_err(|_| "could not read module")?;
    Guest::load(&module, manifest.capabilities.clone())?;
    Ok(Installed { manifest, module })
}
