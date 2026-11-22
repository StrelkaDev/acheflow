use super::Store;
use acheflow_media_api::workspace::Workspace;
use std::path::PathBuf;
pub(crate) struct Directory(pub PathBuf);
impl Directory {
    pub fn new() -> Self {
        let mut bytes = [0; 16];
        openssl::rand::rand_bytes(&mut bytes).unwrap();
        let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("acheflow-workspace-{suffix}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn path(&self) -> PathBuf {
        self.0.join("server.workspace.json")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn extension_host(root: &Directory) -> (std::sync::Arc<crate::control::ControlHost>, String) {
    let (host, workspace) = crate::control::tests::host();
    let id = workspace.id.clone();
    host.runtime
        .attach_store(Store::open(root.path(), "fixture-server").unwrap())
        .unwrap();
    host.runtime.install(workspace, true).await.unwrap();
    host.installed
        .write()
        .unwrap()
        .get_mut("sample")
        .unwrap()
        .manifest
        .settings
        .push(acheflow_extension_api::integration::Setting {
            id: "label".into(),
            label: "Label".into(),
            max_length: 16,
            ..Default::default()
        });
    (host, id)
}

fn extension_edit(
    host: &crate::control::ControlHost,
    workspace: &str,
    suffix: &str,
    id: &str,
    enabled: bool,
    label: Option<&str>,
) -> acheflow_media_api::control::ControlRequest {
    use acheflow_media_api::control::{ControlAction, ControlRequest};
    ControlRequest {
        workspace_id: workspace.into(),
        request_id: format!("{}:{suffix}", crate::control::now_ms()),
        expected_revision: host.snapshot(0, 1).revision,
        action: ControlAction::SetExtension {
            id: id.into(),
            enabled,
            config: label.map(|label| [("label".into(), label.into())].into()),
        },
    }
}

#[tokio::test]
async fn extension_configuration_and_enablement_share_one_durable_revision() {
    let root = Directory::new();
    let (host, id) = extension_host(&root).await;
    let before = host.runtime.saved_state();
    let initial_revision = host.snapshot(0, 1).revision;
    for request in [
        extension_edit(&host, &id, "unknown", "missing", false, None),
        extension_edit(
            &host,
            &id,
            "invalid",
            "sample",
            true,
            Some("a value longer than 16"),
        ),
    ] {
        assert!(host.execute(request).await.is_err());
        assert_eq!(host.runtime.saved_state(), before);
        assert_eq!(runtime(&root).saved_state(), before);
        assert_eq!(host.snapshot(0, 1).revision, initial_revision);
    }
    assert!(host.guests.lock().await.is_empty());

    let enable = extension_edit(&host, &id, "enable", "sample", true, Some("new"));
    let running = host.execute(enable.clone()).await.unwrap();
    assert_eq!(running.revision, initial_revision + 1);
    assert_eq!(
        host.execute(enable).await.unwrap().revision,
        running.revision
    );
    let saved = runtime(&root).saved_state();
    assert_eq!(saved, host.runtime.saved_state());
    assert_eq!(saved.0, before.0 + 1);
    let enabled = &saved.1.as_ref().unwrap().extensions["sample"];
    assert!(enabled.enabled);
    assert_eq!(enabled.config["label"], "new");
    assert!(
        host.services
            .lock()
            .unwrap()
            .instances
            .contains_key("sample")
    );

    let disable = extension_edit(&host, &id, "disable", "sample", false, Some("disabled"));
    let stopped = host.execute(disable).await.unwrap();
    assert_eq!(stopped.revision, running.revision + 1);
    let saved = runtime(&root).saved_state();
    assert_eq!(saved, host.runtime.saved_state());
    let disabled = &saved.1.as_ref().unwrap().extensions["sample"];
    assert!(!disabled.enabled);
    assert_eq!(disabled.config["label"], "disabled");
    assert!(host.guests.lock().await.is_empty());
    assert!(host.services.lock().unwrap().instances.is_empty());
}

#[tokio::test]
async fn extension_load_and_save_failures_preserve_the_last_working_state() {
    let root = Directory::new();
    let (host, id) = extension_host(&root).await;
    let before = host.runtime.saved_state();
    let initial_revision = host.snapshot(0, 1).revision;
    let change_entry = |entry: &str| {
        host.installed
            .write()
            .unwrap()
            .get_mut("sample")
            .unwrap()
            .manifest
            .entry_point = entry.into();
    };
    // Use the real guest with a valid but absent export to fail during instantiation.
    change_entry("missing_export");
    let request = extension_edit(&host, &id, "failed-load", "sample", true, Some("rejected"));
    let failure = host.execute(request.clone()).await.unwrap_err();
    assert_eq!(host.execute(request).await.unwrap_err(), failure);
    assert_eq!(host.runtime.saved_state(), before);
    assert_eq!(runtime(&root).saved_state(), before);
    let unchanged = host.snapshot(0, 1);
    assert_eq!(unchanged.revision, initial_revision);
    assert_eq!(
        unchanged
            .extensions
            .iter()
            .find(|s| s.id == "sample")
            .unwrap()
            .state,
        "stopped"
    );
    assert!(host.guests.lock().await.is_empty());
    assert!(host.services.lock().unwrap().instances.is_empty());

    change_entry("handle");
    let enable = extension_edit(&host, &id, "enable", "sample", true, Some("working"));
    let running = host.execute(enable).await.unwrap();
    let instance = host.services.lock().unwrap().instances["sample"].clone();
    let before = host.runtime.saved_state();

    change_entry("missing_export");
    let replace = extension_edit(&host, &id, "replace", "sample", true, Some("rejected"));
    assert!(host.execute(replace).await.is_err());
    assert_eq!(runtime(&root).saved_state(), before);

    change_entry("handle");
    let backup = root.path().with_extension("fixture-backup");
    std::fs::rename(root.path(), &backup).unwrap();
    std::fs::create_dir(root.path()).unwrap();
    let disable = extension_edit(&host, &id, "failed-save", "sample", false, Some("rejected"));
    assert!(
        host.execute(disable)
            .await
            .unwrap_err()
            .contains("Could not save workspace")
    );
    std::fs::remove_dir(root.path()).unwrap();
    std::fs::rename(backup, root.path()).unwrap();

    assert_eq!(host.runtime.saved_state(), before);
    assert_eq!(runtime(&root).saved_state(), before);
    assert_eq!(host.snapshot(0, 1).revision, running.revision);
    assert!(host.extension_is_live(&id, "sample", &instance));
    let stop = extension_edit(&host, &id, "stop", "sample", false, None);
    host.execute(stop).await.unwrap();
}

#[tokio::test]
async fn cancelling_the_caller_does_not_strand_an_accepted_extension_edit() {
    let root = Directory::new();
    let (host, id) = extension_host(&root).await;
    let request = extension_edit(&host, &id, "enable", "sample", true, Some("new"));
    let guests = host.guests.lock().await;
    let caller = {
        let host = host.clone();
        let request = request.clone();
        tokio::spawn(async move { host.execute(request).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while host.command_lock.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    drop(guests);

    let state = host.execute(request.clone()).await.unwrap();
    assert_eq!(state.revision, request.expected_revision + 1);
    assert_eq!(host.runtime.saved_state(), runtime(&root).saved_state());
    assert!(
        host.services
            .lock()
            .unwrap()
            .instances
            .contains_key("sample")
    );
    let stop = extension_edit(&host, &id, "stop", "sample", false, None);
    host.execute(stop).await.unwrap();
}
#[test]
fn saved_workspace_keeps_identity_and_revision_across_restart() {
    let root = Directory::new();
    let store = Store::open(root.path(), "fixture-server").unwrap();
    let workspace = Workspace {
        edit_revision: 7,
        ..Default::default()
    };
    store.save(7, Some(&workspace)).unwrap();
    drop(store);
    let store = Store::open(root.path(), "fixture-server").unwrap();
    let (revision, saved) = store.load().unwrap();
    assert_eq!(revision, 7);
    assert_eq!(saved.unwrap(), workspace);
}
#[test]
fn corruption_and_a_different_server_never_reset_saved_configuration() {
    let root = Directory::new();
    let store = Store::open(root.path(), "fixture-server").unwrap();
    store
        .save(
            1,
            Some(&Workspace {
                edit_revision: 1,
                ..Default::default()
            }),
        )
        .unwrap();
    drop(store);
    assert!(
        Store::open(root.path(), "another-server")
            .unwrap()
            .load()
            .is_err()
    );
    std::fs::write(root.path(), b"corrupt").unwrap();
    assert!(
        Store::open(root.path(), "fixture-server")
            .unwrap()
            .load()
            .is_err()
    );
    assert_eq!(std::fs::read(root.path()).unwrap(), b"corrupt");
}
#[test]
fn an_interrupted_temporary_write_cannot_replace_the_last_complete_project() {
    let root = Directory::new();
    let store = Store::open(root.path(), "fixture-server").unwrap();
    let workspace = Workspace {
        edit_revision: 1,
        ..Default::default()
    };
    store.save(1, Some(&workspace)).unwrap();
    std::fs::write(
        root.path().with_extension("interrupted.tmp"),
        b"half-written",
    )
    .unwrap();
    assert_eq!(store.load().unwrap().1.unwrap(), workspace);
}

fn runtime(root: &Directory) -> std::sync::Arc<crate::live::Runtime> {
    let runtime = crate::live::Runtime::new(
        std::sync::Arc::new(tokio::sync::Mutex::new(Default::default())),
        "127.0.0.1:0".into(),
    );
    runtime
        .attach_store(Store::open(root.path(), "fixture-server").unwrap())
        .unwrap();
    runtime
}
#[tokio::test]
async fn restart_restores_configuration_but_never_activates_inputs_or_outputs() {
    let root = Directory::new();
    let host = runtime(&root);
    let mut project = Workspace::default();
    project.add_source();
    host.install(project, true).await.unwrap();
    let (revision, saved) = host.saved_state();
    let generation = host
        .control_revision
        .load(std::sync::atomic::Ordering::Acquire);
    assert!(!host.inputs.lock().unwrap().is_empty());
    drop(host);
    let restored = runtime(&root);
    assert_eq!(restored.saved_state(), (revision, saved));
    assert!(restored.inputs.lock().unwrap().is_empty());
    assert!(restored.formats.lock().await.is_empty());
    assert!(!restored.status().await.activated);
    let control = crate::control::ControlHost::new(restored.clone());
    control.snapshot(0, 1);
    assert!(control.guests.lock().await.is_empty());
    let result = crate::client_bridge::call(
        &control,
        &crate::client_bridge::ExecutorCredential {
            id: "fixture".into(),
            workspace_id: restored.saved_state().1.unwrap().id,
            token: String::new(),
        },
        acheflow_extension_api::client_execution::ExecutorRequest::Register {
            registration: acheflow_extension_api::client_execution::ClientRegistration {
                version: 1,
                generation: 1,
                connections: vec![],
            },
        },
    )
    .await;
    assert!(result.unwrap_err().contains("activate the saved workspace"));

    assert_ne!(
        restored
            .control_revision
            .load(std::sync::atomic::Ordering::Acquire),
        generation
    );
}
#[tokio::test]
async fn every_stale_configuration_is_rejected_and_explicit_retry_is_durable() {
    let root = Directory::new();
    let host = runtime(&root);
    let mut project = Workspace::default();
    project.add_source();
    project.add_scene(acheflow_media_api::workspace::Orientation::Horizontal);
    host.install(project, false).await.unwrap();
    let first = host.saved_state().1.unwrap();
    let mut second = first.clone();
    second.name = "Edited by another client".into();
    host.install(second, false).await.unwrap();
    for field in 0..4 {
        let mut stale = first.clone();
        match field {
            0 => stale.sources[0].resolution.width = 640,
            1 => stale.horizontal.fps = 25,
            2 => stale.scenes[0].audio.buses[0].gain = 80,
            _ => stale.scenes[0]
                .layers
                .push(acheflow_media_api::workspace::Layer {
                    source_id: 1,
                    width: 640,
                    height: 360,
                    ..Default::default()
                }),
        }
        assert!(crate::live::validate(&stale).is_ok());
        assert!(
            host.install(stale, false)
                .await
                .unwrap_err()
                .starts_with("conflict:")
        );
    }
    let mut acknowledged = first;
    acknowledged.edit_revision = host.saved_state().0;
    host.install(acknowledged, false).await.unwrap();
    let expected = host.saved_state();
    assert_eq!(runtime(&root).saved_state(), expected);
    let mut stale = expected.1.unwrap();
    host.update_workspace(&stale.id, |w| {
        w.name = "Control edit".into();
        Ok(())
    })
    .unwrap();
    assert!(
        host.install(stale.clone(), false)
            .await
            .unwrap_err()
            .starts_with("conflict:")
    );
    stale.edit_revision = host.saved_state().0;
    let (a, b) = tokio::join!(
        host.install(stale.clone(), false),
        host.install(stale, false)
    );
    assert_ne!(a.is_ok(), b.is_ok());
}
#[tokio::test]
async fn failed_save_and_invalid_configuration_publish_no_runtime_changes() {
    let root = Directory::new();
    let host = runtime(&root);
    let mut project = Workspace::default();
    project.add_source();
    host.install(project, true).await.unwrap();
    let before = host.saved_state();
    let input = host.inputs.lock().unwrap()[&1].clone();
    let generation = host
        .control_revision
        .load(std::sync::atomic::Ordering::Acquire);
    let mut invalid = before.1.clone().unwrap();
    invalid.sources[0].url = "invalid".into();
    assert!(host.install(invalid, true).await.is_err());
    assert_eq!(host.saved_state(), before);
    // Force rename failure without relying on user/root permission behavior.
    let backup = root.0.join("previous.json");
    std::fs::rename(root.path(), &backup).unwrap();
    std::fs::create_dir(root.path()).unwrap();
    let mut changed = before.1.clone().unwrap();
    changed.sources.clear();
    assert!(
        host.install(changed, true)
            .await
            .unwrap_err()
            .contains("save workspace")
    );
    assert_eq!(host.saved_state(), before);
    assert!(std::sync::Arc::ptr_eq(
        &input,
        &host.inputs.lock().unwrap()[&1]
    ));
    assert_eq!(
        host.control_revision
            .load(std::sync::atomic::Ordering::Acquire),
        generation
    );
    std::fs::remove_dir(root.path()).unwrap();
    std::fs::rename(backup, root.path()).unwrap();
    assert_eq!(runtime(&root).saved_state(), before);
}

#[tokio::test]
async fn scene_capability_and_extension_configuration_commands_survive_restart() {
    use acheflow_media_api::control::{ControlAction, ControlRequest};
    let root = Directory::new();
    let (host, workspace) = crate::control::tests::host();
    host.runtime
        .attach_store(Store::open(root.path(), "fixture-server").unwrap())
        .unwrap();
    host.runtime.install(workspace, true).await.unwrap();
    let workspace = host.runtime.saved_state().1.unwrap();
    let scene = &workspace.scenes[0];
    for (index, action) in [
        ControlAction::SetScene {
            scene_id: scene.id,
            spatial: scene.spatial.clone().unwrap(),
        },
        ControlAction::SetClientCapabilities { grants: vec![] },
        ControlAction::SetCallbackProxy {
            url: "https://callbacks.example".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let expected_revision = host.snapshot(0, 1).revision;
        host.execute(ControlRequest {
            workspace_id: workspace.id.clone(),
            request_id: format!("{}:fixture-{index}", crate::control::now_ms()),
            expected_revision,
            action,
        })
        .await
        .unwrap();
        assert_eq!(runtime(&root).saved_state(), host.runtime.saved_state());
    }
    crate::workspace_extensions::update_settings(&host, &workspace.id, "sample", |settings| {
        settings.enabled = true
    })
    .unwrap();
    let restored = runtime(&root);
    assert!(restored.saved_state().1.unwrap().extensions["sample"].enabled);
    assert!(!restored.status().await.activated);
    let restored_host = crate::control::ControlHost::new(restored);
    restored_host.snapshot(0, 1);
    assert!(restored_host.guests.lock().await.is_empty());
}
