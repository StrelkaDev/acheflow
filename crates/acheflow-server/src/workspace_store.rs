#[cfg(test)]
mod tests;
use acheflow_media_api::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) struct Store {
    path: PathBuf,
    server_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    server_id: String,
    revision: u64,
    workspace: Option<Workspace>,
}
impl Store {
    pub fn open(path: PathBuf, server_id: &str) -> Result<Self, String> {
        std::fs::create_dir_all(
            path.parent()
                .ok_or("Invalid workspace storage directory.")?,
        )
        .map_err(|_| "Cannot open workspace storage directory.")?;
        Ok(Self {
            path,
            server_id: server_id.into(),
        })
    }
    pub fn load(&self) -> Result<(u64, Option<Workspace>), String> {
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, None)),
            Err(_) => return Err("Cannot read saved workspace.".into()),
        };
        if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
            return Err("Invalid saved workspace file.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("Saved workspace must have private permissions (0600).".into());
            }
        }
        let bytes = std::fs::read(&self.path).map_err(|_| "Cannot read saved workspace.")?;
        let mut record: Record =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid saved workspace record.")?;
        if record.version != 1 || record.server_id != self.server_id {
            return Err("Saved workspace belongs to a different server or version.".into());
        }
        if let Some(workspace) = &mut record.workspace {
            workspace.migrate_spatial();
            Self::validate(record.revision, workspace)?;
        }
        Ok((record.revision, record.workspace))
    }
    fn validate(revision: u64, workspace: &Workspace) -> Result<(), String> {
        if workspace.version != 1
            || workspace.id.is_empty()
            || workspace.id.len() > 128
            || workspace.edit_revision != revision
            || !workspace.client_extensions.is_empty()
        {
            return Err("Invalid saved workspace configuration.".into());
        }
        crate::live::validate(workspace)
    }
    pub fn save(&self, revision: u64, workspace: Option<&Workspace>) -> Result<(), String> {
        if let Some(workspace) = workspace {
            Self::validate(revision, workspace)?;
        }
        let bytes = serde_json::to_vec(&Record {
            version: 1,
            server_id: self.server_id.clone(),
            revision,
            workspace: workspace.cloned(),
        })
        .map_err(|_| "Cannot encode workspace.")?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Saved workspace exceeds 2 MiB.".into());
        }
        atomic_write(&self.path, &bytes)
    }
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut random = [0; 16];
    openssl::rand::rand_bytes(&mut random).map_err(|_| "Cannot create workspace save.")?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = path.with_extension(format!("{suffix}.tmp"));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Cannot create workspace save.")?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map_err(|_| "Could not save workspace; configuration was not changed.".into())
}
