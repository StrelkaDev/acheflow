//! Read installation artifacts on the host; guests have no filesystem capability.
use acheflow_control::extensions::{Guest, GuestManifest};
use std::{collections::BTreeMap, path::Path};

pub struct Installed {
    pub manifest: GuestManifest,
    pub module: Vec<u8>,
}
#[derive(Default)]
pub struct Discovery {
    pub installed: BTreeMap<String, Installed>,
    pub errors: Vec<String>,
}
pub fn discover() -> BTreeMap<String, Installed> {
    let root = std::env::var_os("ACHEFLOW_EXTENSIONS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let local = std::path::PathBuf::from("extensions");
            if local.is_dir() {
                local
            } else {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| {
                        let directory = p.parent()?;
                        [
                            directory.join("extensions"),
                            directory.join("../Resources/extensions"),
                        ]
                        .into_iter()
                        .find(|root| root.is_dir())
                    })
                    .unwrap_or(local)
            }
        });
    discover_at(&root)
}
pub fn discover_at(root: &Path) -> BTreeMap<String, Installed> {
    let discovery = discover_catalog_at(root);
    for error in discovery.errors {
        crate::desktop::log(format_args!("Extension installation skipped: {error}"));
    }
    discovery.installed
}
pub fn discover_catalog_at(root: &Path) -> Discovery {
    let mut result = Discovery::default();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => {
            result.errors.push(format!(
                "{}: extension directory is unavailable",
                root.display()
            ));
            return result;
        }
    };
    let Ok(mut directories) = entries
        .take(65)
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
    else {
        result.errors.push(format!(
            "{}: extension directory could not be fully read; catalog disabled",
            root.display()
        ));
        return result;
    };
    if directories.len() > 64 {
        result.errors.push(format!(
            "{}: extension directory exceeds scan limit; catalog disabled",
            root.display()
        ));
        return result;
    }
    directories.sort();
    let mut sources = BTreeMap::<String, Vec<String>>::new();
    for directory in directories {
        if !directory.join("manifest.json").is_file() {
            continue;
        }
        let extension = match load(&directory) {
            Ok(Some(extension)) => extension,
            Ok(None) => continue,
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        let id = &extension.manifest.id;
        let locations = sources.entry(id.clone()).or_default();
        locations.push(directory.display().to_string());
        if locations.len() > 1 {
            result.installed.remove(id);
            result.errors.push(format!(
                "conflicting extension ID '{id}'; all installations disabled: {}",
                locations.join(", ")
            ));
            continue;
        }
        if result.installed.len() >= 8 {
            result.errors.push(format!(
                "{}: server catalog limit reached",
                directory.display()
            ));
            continue;
        }
        result.installed.insert(id.clone(), extension);
    }
    result
}

fn load(directory: &Path) -> Result<Option<Installed>, String> {
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
    if manifest.execution_side == acheflow_extension_api::integration::ExecutionSide::Client {
        return Ok(None);
    }
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
    Ok(Some(Installed { manifest, module }))
}

#[cfg(test)]
#[path = "extension_registry_tests.rs"]
mod tests;
