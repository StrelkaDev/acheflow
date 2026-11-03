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
    update_settings(host, owner, id, |settings| settings.config = config.clone())
}
/// Write-only credentials never pass through workspace configuration or guest events.
pub async fn set_secret(
    host: &Arc<ControlHost>,
    owner: &str,
    id: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    if !host.owns(owner) {
        return Err("conflict: workspace is not active".into());
    }
    let extension = host.installed.get(id).ok_or("not_found: extension")?;
    let setting = extension
        .manifest
        .settings
        .iter()
        .find(|s| s.id == key && s.secret)
        .ok_or("invalid_argument: undeclared secret field")?;
    if value.len() > setting.max_length {
        return Err("invalid_argument: secret value is too long".into());
    }
    let namespace = (owner.to_owned(), id.to_owned());
    let has_capacity = |secrets: &std::collections::BTreeMap<
        (String, String),
        crate::extension_transport::Secrets,
    >| {
        (secrets.contains_key(&namespace) || secrets.len() < 256)
            && secrets
                .get(&namespace)
                .is_none_or(|s| s.contains_key(key) || s.len() < 32)
    };
    {
        let secrets = host.secrets.lock().await;
        if !value.is_empty() && !has_capacity(&secrets) {
            return Err("invalid_argument: credential storage limit reached".into());
        }
    }
    let enabled = host
        .runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .filter(|w| w.id == owner)
        .and_then(|w| w.extensions.get(id))
        .is_some_and(|s| s.enabled);
    // Cancel before replacing credentials: late token captures from an old
    // instance must not overwrite a manual update or restore a removed secret.
    if enabled {
        extension_host::set_enabled(host.clone(), owner, id, false).await?;
    }
    let result = {
        let mut secrets = host.secrets.lock().await;
        if !value.is_empty() && !has_capacity(&secrets) {
            Err("invalid_argument: credential storage limit reached".into())
        } else {
            if value.is_empty() {
                if let Some(values) = secrets.get_mut(&namespace) {
                    values.remove(key);
                    if values.is_empty() {
                        secrets.remove(&namespace);
                    }
                }
            } else {
                secrets
                    .entry(namespace.clone())
                    .or_default()
                    .insert(key.into(), value.into());
            }
            Ok(())
        }
    };
    if enabled {
        extension_host::set_enabled(host.clone(), owner, id, true).await?;
    }
    result
}
fn reject_public_secrets(host: &ControlHost, workspace: &Workspace) -> Result<(), String> {
    for (id, settings) in &workspace.extensions {
        if let Some(extension) = host.installed.get(id)
            && extension
                .manifest
                .settings
                .iter()
                .any(|s| s.secret && settings.config.contains_key(&s.id))
        {
            return Err("Secret fields must be saved separately on the server.".into());
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
    reject_public_secrets(&host, &workspace).map_err(|error| {
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
    reject_public_secrets(&host, &request.workspace).map_err(|error| {
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

    #[tokio::test]
    async fn manual_secrets_are_scoped_redacted_and_rejected_from_public_settings() {
        let (host, workspace) = crate::control::tests::host();
        let secret = "fixture-token-not-a-real-credential";
        let mut events = host.events.subscribe();
        let make_request = |id: &str, value: &str| control::ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!(
                "{}:{}",
                crate::control::now_ms(),
                acheflow_media_api::workspace::new_token()
            ),
            expected_revision: host.snapshot(0, 1).revision,
            action: control::ControlAction::SetExtensionSecret {
                id: id.into(),
                key: "access_token".into(),
                value: control::SecretValue(value.into()),
                config: None,
            },
        };
        let request = make_request("youtube", secret);
        assert!(!format!("{request:?}").contains(secret));
        let snapshot = host.execute(request).await.unwrap();
        assert!(!serde_json::to_string(&snapshot).unwrap().contains(secret));
        assert_eq!(
            snapshot
                .extensions
                .iter()
                .find(|s| s.id == "youtube")
                .unwrap()
                .configured_secrets,
            ["access_token"]
        );
        assert!(
            snapshot
                .extensions
                .iter()
                .find(|s| s.id == "vklive")
                .unwrap()
                .configured_secrets
                .is_empty()
        );
        assert_eq!(
            host.secrets
                .lock()
                .await
                .get(&(workspace.id.clone(), "youtube".into()))
                .unwrap()["access_token"],
            secret
        );
        assert!(
            events.try_recv().is_err(),
            "credentials must never enter the event bus"
        );
        assert!(
            configure_extension(
                &host,
                &workspace.id,
                "youtube",
                &std::collections::BTreeMap::from([("access_token".into(), secret.into())])
            )
            .is_err()
        );
        let snapshot = host.execute(make_request("youtube", "")).await.unwrap();
        assert!(
            snapshot
                .extensions
                .iter()
                .find(|s| s.id == "youtube")
                .unwrap()
                .configured_secrets
                .is_empty()
        );
        assert!(
            !host
                .secrets
                .lock()
                .await
                .get(&(workspace.id.clone(), "youtube".into()))
                .is_some_and(|s| s.contains_key("access_token"))
        );
        assert!(host.execute(make_request("sample", secret)).await.is_err());
    }

    #[tokio::test]
    async fn replacing_credentials_restarts_the_real_guest_and_rejects_its_old_instance() {
        let (mut host, workspace) = crate::control::tests::host();
        Arc::get_mut(&mut host)
            .unwrap()
            .installed
            .get_mut("sample")
            .unwrap()
            .manifest
            .settings
            .push(acheflow_extension_api::integration::Setting {
                id: "credential".into(),
                label: "Fixture credential".into(),
                secret: true,
                ..Default::default()
            });
        let request = |action| control::ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!(
                "{}:{}",
                crate::control::now_ms(),
                acheflow_media_api::workspace::new_token()
            ),
            expected_revision: host.snapshot(0, 1).revision,
            action,
        };
        host.execute(request(control::ControlAction::SetExtension {
            id: "sample".into(),
            enabled: true,
            config: None,
        }))
        .await
        .unwrap();
        wait_for_panel(&host).await;
        let old = host.services.lock().unwrap().instances["sample"].clone();
        host.execute(request(control::ControlAction::SetExtensionSecret {
            id: "sample".into(),
            key: "credential".into(),
            value: control::SecretValue("fixture-token".into()),
            config: None,
        }))
        .await
        .unwrap();
        wait_for_panel(&host).await;
        assert_ne!(old, host.services.lock().unwrap().instances["sample"]);
        assert!(
            host.publish_from(
                &workspace.id,
                "sample",
                &old,
                serde_json::json!({"type":"failure","message":"stale"}),
                0
            )
            .await
            .is_err()
        );
        host.execute(request(control::ControlAction::SetExtension {
            id: "sample".into(),
            enabled: false,
            config: None,
        }))
        .await
        .unwrap();
    }

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

        let _ = configure(State(host.clone()), Json(first.clone()))
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

    #[tokio::test]
    async fn secrets_are_scoped_to_workspaces_and_inactive_instances_cannot_publish() {
        let (host, workspace) = crate::control::tests::host();
        host.secrets.lock().await.insert(
            ("another-workspace".into(), "sample".into()),
            std::collections::BTreeMap::from([("credential".into(), "private".into())]),
        );
        let snapshot = host
            .execute(control::ControlRequest {
                workspace_id: workspace.id.clone(),
                request_id: format!("{}:start", crate::control::now_ms()),
                expected_revision: 0,
                action: control::ControlAction::SetExtension {
                    id: "sample".into(),
                    enabled: true,
                    config: None,
                },
            })
            .await
            .unwrap();
        assert!(snapshot.extension_settings["sample"].enabled);
        let instance = host.services.lock().unwrap().instances["sample"].clone();
        host.execute(control::ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:stop", crate::control::now_ms()),
            expected_revision: snapshot.revision,
            action: control::ControlAction::SetExtension {
                id: "sample".into(),
                enabled: false,
                config: None,
            },
        })
        .await
        .unwrap();
        assert!(
            host.publish_from(
                &workspace.id,
                "sample",
                &instance,
                serde_json::json!({"type":"panel","text":"late"}),
                0
            )
            .await
            .is_err()
        );
        assert!(
            !host
                .secrets
                .lock()
                .await
                .contains_key(&(workspace.id, "sample".into()))
        );
    }
}
