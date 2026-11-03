//! Apply workspace preferences to the extension lifecycle at the live API boundary.
use crate::{control::ControlHost, extension_host, live};
use acheflow_media_api::{
    live::{ControlRequest, StartRequest},
    workspace::Workspace,
};
use axum::{
    Json, Router,
    extract::State,
    routing::{post, put},
};
use std::sync::Arc;

pub fn routes(host: Arc<ControlHost>) -> Router {
    Router::new()
        .route("/api/live/config", put(configure).delete(deactivate))
        .route("/api/live/start", post(start))
        .with_state(host)
}

pub fn update_settings(
    host: &ControlHost,
    owner: &str,
    id: &str,
    update: impl FnOnce(&mut acheflow_media_api::workspace::ExtensionSettings),
) -> Result<(), String> {
    let mut workspace = host.runtime.workspace.write().unwrap();
    let workspace = workspace
        .as_mut()
        .filter(|w| w.id == owner)
        .ok_or("conflict: workspace changed")?;
    update(workspace.extensions.entry(id.into()).or_default());
    workspace.edit_revision = host
        .runtime
        .control_revision
        .load(std::sync::atomic::Ordering::Acquire)
        + 1;
    Ok(())
}
pub fn configure_extension(
    host: &ControlHost,
    owner: &str,
    id: &str,
    config: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    configure_public_settings(host, id, config)?;
    update_settings(host, owner, id, |settings| settings.config = config.clone())
}
fn configure_public_settings(
    host: &ControlHost,
    id: &str,
    config: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let extension = host.installed.get(id).ok_or("not_found: extension")?;
    if config.len() > 32
        || config.iter().any(|(key, value)| {
            extension
                .manifest
                .settings
                .iter()
                .find(|s| s.id == *key)
                .is_none_or(|s| s.secret || value.len() > s.max_length)
        })
    {
        return Err("invalid extension settings".into());
    }
    Ok(())
}

fn validate_public_settings(host: &ControlHost, workspace: &Workspace) -> Result<(), String> {
    if !workspace.client_extensions.is_empty() {
        return Err("client integration settings must stay in the client catalog".into());
    }
    acheflow_control::client_execution::ExecutionRegistry::default().set_policy(
        &workspace.id,
        workspace.client_capabilities.clone(),
        crate::control::now_ms(),
    )?;
    for (id, settings) in &workspace.extensions {
        if host.installed.contains_key(id) {
            configure_public_settings(host, id, &settings.config)?;
        } else if !settings.config.is_empty() {
            return Err("unknown extension settings cannot be sent to the server".into());
        }
    }
    Ok(())
}
async fn reconcile(host: &Arc<ControlHost>) {
    let workspace = host.runtime.workspace.read().unwrap().clone();
    let owner = workspace.as_ref().map(|w| w.id.as_str());
    // Snapshot reads can already have reset Services to the new workspace.
    let previous = host.extension_owner.lock().unwrap().clone();
    if previous.as_deref() != owner {
        for id in host.installed.keys() {
            let _ = extension_host::set_enabled(
                host.clone(),
                previous.as_deref().unwrap_or_default(),
                id,
                false,
            )
            .await;
        }
    }
    let snapshot = host.snapshot(0, 1);
    *host.extension_owner.lock().unwrap() = owner.map(str::to_owned);
    let Some(workspace) = workspace else {
        return;
    };
    for status in snapshot.extensions {
        let enabled = workspace
            .extensions
            .get(&status.id)
            .is_some_and(|e| e.enabled);
        if enabled == (status.state != "stopped") {
            continue;
        }
        if let Err(error) =
            extension_host::set_enabled(host.clone(), &workspace.id, &status.id, enabled).await
        {
            extension_host::failed(host, &workspace.id, &status.id, error);
        }
    }
}

pub async fn configure(
    State(host): State<Arc<ControlHost>>,
    workspace: Json<Workspace>,
) -> live::ApiResult {
    let _command = host.command_lock.lock().await;
    validate_public_settings(&host, &workspace).map_err(|error| {
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(acheflow_media_api::live::ApiError { error }),
        )
    })?;
    let result = live::configure(State(host.runtime.clone()), workspace).await;
    if result.is_ok() {
        reconcile(&host).await;
    }
    result
}
pub async fn start(
    State(host): State<Arc<ControlHost>>,
    request: Json<StartRequest>,
) -> live::ApiResult {
    let _command = host.command_lock.lock().await;
    validate_public_settings(&host, &request.workspace).map_err(|error| {
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(acheflow_media_api::live::ApiError { error }),
        )
    })?;
    let result = live::start(State(host.runtime.clone()), request).await;
    // A compositor failure can occur after applying the workspace.
    reconcile(&host).await;
    result
}
pub async fn deactivate(
    State(host): State<Arc<ControlHost>>,
    request: Json<ControlRequest>,
) -> live::ApiResult {
    let _command = host.command_lock.lock().await;
    let result = live::deactivate(State(host.runtime.clone()), request).await;
    if result.is_ok() {
        reconcile(&host).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::{control, workspace::ExtensionSettings};

    async fn wait_for_panel(host: &ControlHost) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while host.snapshot(0, 1).panels.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn applying_switching_and_deactivating_restore_only_the_selected_extensions() {
        let (host, mut first) = crate::control::tests::host();
        first.extensions.insert(
            "sample".into(),
            ExtensionSettings {
                enabled: true,
                ..Default::default()
            },
        );
        let _ = configure(State(host.clone()), Json(first)).await.unwrap();
        wait_for_panel(&host).await;
        let first = host.runtime.workspace.read().unwrap().clone().unwrap();
        let panel = serde_json::to_value(&host.snapshot(0, 1).panels).unwrap();
        let _ = configure(State(host.clone()), Json(first.clone()))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&host.snapshot(0, 1).panels).unwrap(),
            panel
        );

        let second = Workspace::default();
        let _ = configure(State(host.clone()), Json(second.clone()))
            .await
            .unwrap();
        assert!(host.guests.lock().await.is_empty());
        assert!(host.snapshot(0, 1).panels.is_empty());
        assert!(host.snapshot(0, 1).extension_settings.is_empty());

        let _ = configure(State(host.clone()), Json(first.clone()))
            .await
            .unwrap();
        wait_for_panel(&host).await;
        let revision = host.snapshot(0, 1).revision;
        host.execute(control::ControlRequest {
            workspace_id: first.id.clone(),
            request_id: format!("{}:disable", crate::control::now_ms()),
            expected_revision: revision,
            action: control::ControlAction::SetExtension {
                id: "sample".into(),
                enabled: false,
                config: None,
            },
        })
        .await
        .unwrap();
        let saved = host.runtime.workspace.read().unwrap().clone().unwrap();
        assert!(!saved.extensions["sample"].enabled);
        let _ = configure(State(host.clone()), Json(second)).await.unwrap();
        let _ = configure(State(host.clone()), Json(saved)).await.unwrap();
        assert!(host.guests.lock().await.is_empty());

        // Re-enabling is a new edit based on the current acknowledged revision.
        let mut reenabled = first.clone();
        reenabled.edit_revision = host
            .runtime
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .edit_revision;
        let _ = configure(State(host.clone()), Json(reenabled))
            .await
            .unwrap();
        wait_for_panel(&host).await;
        let _ = deactivate(
            State(host.clone()),
            Json(ControlRequest {
                workspace_id: first.id,
            }),
        )
        .await
        .unwrap();
        assert!(host.guests.lock().await.is_empty());
        assert!(host.snapshot(0, 1).panels.is_empty());
    }

    #[tokio::test]
    async fn snapshot_reads_during_workspace_switch_do_not_hide_old_running_guests() {
        let (host, mut first) = crate::control::tests::host();
        first.extensions.entry("sample".into()).or_default().enabled = true;
        let _ = configure(State(host.clone()), Json(first)).await.unwrap();
        wait_for_panel(&host).await;
        // The UI may poll a snapshot after Runtime::apply but before reconciliation.
        let _ = live::configure(State(host.runtime.clone()), Json(Workspace::default()))
            .await
            .unwrap();
        host.snapshot(0, 1);
        reconcile(&host).await;
        assert!(host.guests.lock().await.is_empty());
        assert!(host.snapshot(0, 1).panels.is_empty());
    }
}

#[cfg(test)]
mod credential_boundary_tests {
    use super::*;
    #[tokio::test]
    async fn workspace_upload_rejects_removed_token_fields_and_unknown_extension_config() {
        let (host, mut workspace) = crate::control::tests::host();
        workspace
            .extensions
            .entry("youtube".into())
            .or_default()
            .config
            .insert("access_token".into(), "private".into());
        let (status, _) = configure(State(host.clone()), Json(workspace.clone()))
            .await
            .unwrap_err();
        assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !host
                .runtime
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .extensions
                .contains_key("youtube")
        );
        workspace.extensions.clear();
        workspace
            .extensions
            .entry("unknown-provider".into())
            .or_default()
            .config
            .insert("token".into(), "private".into());
        assert!(configure(State(host), Json(workspace)).await.is_err());
    }
}

#[cfg(test)]
mod persistence_revision_tests {
    use super::*;
    use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
    use acheflow_media_api::control::{ControlAction, ControlRequest};
    #[tokio::test]
    async fn stale_workspace_apply_cannot_erase_remote_capability_policy_or_extension_settings() {
        let (mut host, mut stale) = crate::control::tests::host();
        std::sync::Arc::get_mut(&mut host)
            .unwrap()
            .installed
            .get_mut("sample")
            .unwrap()
            .manifest
            .settings
            .push(acheflow_extension_api::integration::Setting {
                id: "application".into(),
                label: "Application".into(),
                ..Default::default()
            });
        stale.migrate_spatial();
        *host.runtime.workspace.write().unwrap() = Some(stale.clone());
        let grants = vec![CapabilityGrant {
            provider: "youtube".into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }];
        host.execute(ControlRequest {
            workspace_id: stale.id.clone(),
            request_id: format!("{}:policy", crate::control::now_ms()),
            expected_revision: 0,
            action: ControlAction::SetClientCapabilities {
                grants: grants.clone(),
            },
        })
        .await
        .unwrap();
        assert!(
            configure(State(host.clone()), Json(stale.clone()))
                .await
                .is_err(),
            "stale Apply silently erased policy"
        );
        assert_eq!(
            host.runtime
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .client_capabilities,
            grants
        );
        let stale = host.runtime.workspace.read().unwrap().clone().unwrap();
        let revision = host.snapshot(0, 1).revision;
        host.execute(ControlRequest {
            workspace_id: stale.id.clone(),
            request_id: format!("{}:settings", crate::control::now_ms()),
            expected_revision: revision,
            action: ControlAction::SetExtension {
                id: "sample".into(),
                enabled: true,
                config: Some(std::collections::BTreeMap::from([(
                    "application".into(),
                    "publicvideo".into(),
                )])),
            },
        })
        .await
        .unwrap();
        assert!(
            configure(State(host.clone()), Json(stale)).await.is_err(),
            "stale Apply silently erased extension settings"
        );
        let current = host.runtime.workspace.read().unwrap().clone().unwrap();
        assert_eq!(
            current.extensions["sample"].config["application"],
            "publicvideo"
        );
        let _ = configure(State(host.clone()), Json(current)).await.unwrap();
        assert_eq!(
            host.snapshot(0, 1).extension_settings["sample"].config["application"],
            "publicvideo"
        );
    }
}

#[cfg(test)]
mod client_setting_boundary_tests {
    use super::*;
    #[tokio::test]
    async fn server_rejects_local_client_settings_instead_of_storing_them() {
        let (host, mut local) = crate::control::tests::host();
        local.migrate_spatial();
        local.client_extensions.insert(
            "client-only".into(),
            acheflow_media_api::workspace::ExtensionSettings {
                enabled: true,
                config: std::collections::BTreeMap::from([(
                    "application".into(),
                    "local-only-app".into(),
                )]),
            },
        );
        assert!(
            configure(State(host.clone()), Json(local.clone()))
                .await
                .is_err()
        );
        assert!(
            host.runtime
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .client_extensions
                .is_empty()
        );
        let _ = configure(State(host.clone()), Json(local.server_configuration()))
            .await
            .unwrap();
        assert!(
            host.runtime
                .workspace
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .client_extensions
                .is_empty()
        );
        assert!(
            !host
                .snapshot(0, 1)
                .extension_settings
                .contains_key("client-only")
        );
    }
}
