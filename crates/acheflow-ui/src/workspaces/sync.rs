//! Durable merge bases are local catalog metadata, excluded from workspace exports.
use super::*;
use acheflow_media_api::audio::{MixerAck, MixerEdit, MixerRequest};
use acheflow_media_api::server_workspace::WorkspaceState;
use acheflow_media_api::workspace::Orientation;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Replica {
    server: String,
    workspace: Workspace,
    editable: bool,
    conflict: bool,
}
pub(crate) struct Update {
    pub project: Workspace,
    pub conflict: bool,
}
fn configuration(workspace: &Workspace) -> Workspace {
    let mut value = workspace.server_configuration();
    value.server_url.clear();
    value.edit_revision = 0;
    value.migrate_spatial();
    value
}
// Arrays are atomic edit units. Conflicting lists remain local for explicit resolution.
fn merge(
    base: &serde_json::Value,
    local: &serde_json::Value,
    remote: &serde_json::Value,
    conflict: &mut bool,
) -> serde_json::Value {
    if local == base || local == remote {
        return remote.clone();
    }
    if remote == base {
        return local.clone();
    }
    if let (Some(base), Some(local), Some(remote)) =
        (base.as_object(), local.as_object(), remote.as_object())
    {
        let keys: std::collections::BTreeSet<_> = base
            .keys()
            .chain(local.keys())
            .chain(remote.keys())
            .collect();
        let mut result = serde_json::Map::new();
        for key in keys {
            let (b, l, r) = (base.get(key), local.get(key), remote.get(key));
            let value = if l == b || l == r {
                r.cloned()
            } else if r == b {
                l.cloned()
            } else if let (Some(b), Some(l), Some(r)) = (b, l, r) {
                Some(merge(b, l, r, conflict))
            } else {
                *conflict = true;
                l.cloned()
            };
            if let Some(value) = value {
                result.insert(key.clone(), value);
            }
        }
        return result.into();
    }
    *conflict = true;
    local.clone()
}
fn reconcile(
    current: &Workspace,
    remote: &Workspace,
    previous: Option<&Replica>,
    editable: bool,
) -> Result<(Workspace, bool), String> {
    let mut conflict = previous.is_some_and(|r| r.conflict);
    let config = if !editable || previous.is_some_and(|r| !r.editable) {
        configuration(remote)
    } else if let Some(previous) = previous {
        if previous.workspace == *remote {
            configuration(current)
        } else {
            serde_json::from_value(merge(
                &serde_json::to_value(configuration(&previous.workspace))
                    .map_err(|_| "Cannot compare workspace.")?,
                &serde_json::to_value(configuration(current))
                    .map_err(|_| "Cannot compare workspace.")?,
                &serde_json::to_value(configuration(remote))
                    .map_err(|_| "Cannot compare workspace.")?,
                &mut conflict,
            ))
            .map_err(|_| "Cannot merge workspace.")?
        }
    } else {
        conflict = configuration(current) != configuration(remote)
            && current.edit_revision != remote.edit_revision;
        configuration(current)
    };
    // Independent field edits can still conflict through references (for example,
    // one client removes an input while another adds a scene using it).
    if editable
        && previous.is_some_and(|r| r.workspace.edit_revision != remote.edit_revision)
        && current.validate().is_ok()
        && remote.validate().is_ok()
        && config.validate().is_err()
    {
        return Ok((configuration(current), true));
    }
    Ok((config, conflict))
}
impl Catalog {
    pub(crate) fn editable_base(&self, local: &Workspace) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|record| {
                record.id == self.active
                    && record.config.id == local.id
                    && record.config.server_url == local.server_url
            })
            .and_then(|record| record.replica.as_ref())
            .filter(|replica| replica.editable && !replica.conflict)
            .map(|replica| &replica.workspace)
    }

    pub(crate) fn applied_mixer(
        &mut self,
        local: &Workspace,
        request: &MixerRequest,
        ack: MixerAck,
        server_id: &str,
    ) -> bool {
        self.applied_delta(
            local,
            request.expected_revision,
            ack.revision,
            server_id,
            |base| {
                for edit in &request.edits {
                    edit.apply(base)?;
                }
                Ok(())
            },
        )
    }

    pub(crate) fn applied_delta(
        &mut self,
        local: &Workspace,
        expected: u64,
        revision: u64,
        server_id: &str,
        apply: impl FnOnce(&mut Workspace) -> Result<(), String>,
    ) -> bool {
        let Some(replica) = self
            .workspaces
            .iter_mut()
            .find(|record| {
                record.id == self.active
                    && record.config.id == local.id
                    && record.config.server_url == local.server_url
            })
            .and_then(|record| record.replica.as_mut())
            .filter(|replica| {
                replica.server == server_id
                    && replica.editable
                    && !replica.conflict
                    && replica.workspace.edit_revision == expected
            })
        else {
            return false;
        };
        if expected.checked_add(1) != Some(revision) {
            return false;
        }
        let mut base = replica.workspace.clone();
        if apply(&mut base).is_err() {
            return false;
        }
        base.edit_revision = revision;
        replica.workspace = base;
        true
    }

    pub(crate) fn can_take_scene(
        &self,
        local: &Workspace,
        orientation: Orientation,
        scene: u32,
    ) -> bool {
        self.workspaces
            .iter()
            .find(|record| {
                record.id == self.active
                    && record.config.id == local.id
                    && record.config.server_url == local.server_url
            })
            .and_then(|record| record.replica.as_ref())
            .is_some_and(|replica| {
                let server = &replica.workspace;
                if !replica.editable
                    || replica.conflict
                    || local.stream(orientation).base != server.stream(orientation).base
                {
                    return false;
                }
                let Some(draft) = local
                    .scenes
                    .iter()
                    .find(|entry| entry.id == scene && entry.orientation == orientation)
                else {
                    return false;
                };
                let Some(saved) = server.scenes.iter().find(|saved| saved.id == scene) else {
                    return false;
                };
                if draft.audio != saved.audio
                    || !acheflow_media_api::live::SceneContent::unchanged(
                        draft,
                        saved,
                        &server.stream(orientation).base,
                    )
                {
                    return false;
                }
                draft.visual_layers().all(|layer| {
                    local
                        .sources
                        .iter()
                        .find(|source| source.id == layer.source_id)
                        == server
                            .sources
                            .iter()
                            .find(|source| source.id == layer.source_id)
                })
            })
    }

    pub(crate) fn mixer_has_target(&self, local: &Workspace, edit: &MixerEdit) -> bool {
        self.workspaces
            .iter()
            .find(|record| {
                record.id == self.active
                    && record.config.id == local.id
                    && record.config.server_url == local.server_url
            })
            .and_then(|record| record.replica.as_ref())
            .is_some_and(|replica| {
                replica.editable
                    && replica
                        .workspace
                        .scenes
                        .iter()
                        .any(|entry| entry.id == edit.scene_id())
                    && match edit {
                        MixerEdit::Strip { key, .. } => {
                            replica.workspace.sources.iter().any(|source| {
                                source.id == key.source_id && source.kind.has_audio_input()
                            })
                        }
                        MixerEdit::Bus { .. } => true,
                    }
            })
    }

    pub(crate) fn sync_server(
        &mut self,
        local: &Workspace,
        state: &WorkspaceState,
    ) -> Result<Option<Update>, String> {
        let editable = state.workspace.is_some();
        let Some(remote) = state.workspace.as_ref().or(state.view.as_ref()) else {
            return Ok(None);
        };
        if remote.edit_revision != state.revision
            || state.summary.as_ref().is_none_or(|s| s.id != remote.id)
        {
            return Err("Invalid server workspace version.".into());
        }
        let endpoint = crate::media::server_url(local, "")?;
        let index = self.workspaces.iter().position(|r| {
            r.config.id == remote.id
                && crate::media::server_url(&r.config, "").ok().as_ref() == Some(&endpoint)
        });
        if let Some(index) = index
            && self.workspaces[index].replica.as_ref().is_some_and(|r| {
                r.server == state.server_id && r.workspace.edit_revision > state.revision
            })
        {
            return Ok(None);
        }
        self.update(local.clone());
        let index = match index {
            Some(index) => index,
            None => {
                let mut project = remote.clone();
                project.server_url = local.server_url.clone();
                self.add(project);
                self.workspaces.len() - 1
            }
        };
        let record = &mut self.workspaces[index];
        let previous = record
            .replica
            .as_ref()
            .filter(|r| r.server == state.server_id);
        let current = record.config.clone();
        let (config, mut conflict) = reconcile(&current, remote, previous, editable)?;
        let mut project = config;
        project.server_url = local.server_url.clone();
        project.client_extensions = current.client_extensions;
        project.client_callback_proxy_url = current.client_callback_proxy_url;
        project.edit_revision = if editable && conflict {
            current.edit_revision
        } else {
            state.revision
        };
        if !editable {
            conflict = false;
        }
        record.config = project.clone();
        record.replica = Some(Replica {
            server: state.server_id.clone(),
            workspace: remote.clone(),
            editable,
            conflict,
        });
        self.active = record.id;
        Ok(Some(Update { project, conflict }))
    }
    pub(crate) fn resolve_server(&mut self, local: &Workspace, use_server: bool) -> Option<Update> {
        let record = self
            .workspaces
            .iter_mut()
            .find(|r| r.id == self.active && r.config.id == local.id)?;
        let replica = record.replica.as_mut()?;
        let mut project = if use_server {
            replica.workspace.clone()
        } else {
            local.clone()
        };
        project.server_url = local.server_url.clone();
        project.client_extensions = local.client_extensions.clone();
        project.client_callback_proxy_url = local.client_callback_proxy_url.clone();
        project.edit_revision = replica.workspace.edit_revision;
        replica.conflict = false;
        record.config = project.clone();
        Some(Update {
            project,
            conflict: false,
        })
    }
    pub(crate) fn applied_server(
        &mut self,
        local: &Workspace,
        revision: u64,
        server_id: &str,
    ) -> bool {
        let Some(record) = self.workspaces.iter_mut().find(|r| {
            r.id == self.active
                && r.config.id == local.id
                && r.config.server_url == local.server_url
        }) else {
            return false;
        };
        if record
            .replica
            .as_ref()
            .is_some_and(|r| r.server == server_id && r.workspace.edit_revision > revision)
        {
            return false;
        }
        let replica = record.replica.get_or_insert_with(|| Replica {
            server: server_id.into(),
            workspace: local.server_configuration(),
            editable: true,
            conflict: false,
        });
        if replica.server != server_id || !replica.editable {
            return false;
        }
        replica.workspace = local.server_configuration();
        replica.workspace.server_url.clear();
        replica.workspace.edit_revision = revision;
        replica.conflict = false;
        true
    }
}
#[cfg(test)]
mod tests;
