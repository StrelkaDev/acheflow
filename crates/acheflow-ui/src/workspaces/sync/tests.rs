use super::*;
fn state(project: Workspace, editable: bool) -> WorkspaceState {
    WorkspaceState {
        server_id: "fixture".into(),
        revision: project.edit_revision,
        summary: Some(acheflow_media_api::server_workspace::WorkspaceSummary {
            id: project.id.clone(),
            name: project.name.clone(),
        }),
        workspace: editable.then(|| project.clone()),
        view: (!editable).then_some(project),
    }
}
fn remote(revision: u64) -> Workspace {
    Workspace {
        id: "shared-fixture".into(),
        edit_revision: revision,
        ..Default::default()
    }
}
#[test]
fn two_independent_catalogs_attach_without_destroying_their_local_drafts() {
    let server = state(remote(1), true);
    for _ in 0..2 {
        let mut catalog = Catalog::default();
        let old = catalog.current().clone();
        let update = catalog.sync_server(&old, &server).unwrap().unwrap();
        assert_eq!(update.project.id, "shared-fixture");
        assert!(!update.conflict);
        assert_eq!(catalog.workspaces.len(), 2);
        assert_eq!(catalog.workspaces[0].config, old);
    }
}
#[test]
fn unrelated_remote_updates_merge_and_same_field_conflicts_remain_until_resolved() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut local = catalog
        .sync_server(&old, &state(remote(1), true))
        .unwrap()
        .unwrap()
        .project;
    local.name = "Local draft".into();
    let mut changed = remote(2);
    changed.horizontal.fps = 25;
    let update = catalog
        .sync_server(&local, &state(changed.clone(), true))
        .unwrap()
        .unwrap();
    assert!(!update.conflict);
    assert_eq!(update.project.name, "Local draft");
    assert_eq!(update.project.horizontal.fps, 25);
    changed.edit_revision = 3;
    changed.name = "Other editor".into();
    let update = catalog
        .sync_server(&update.project, &state(changed.clone(), true))
        .unwrap()
        .unwrap();
    assert!(update.conflict);
    assert_eq!(update.project.edit_revision, 2);
    assert_eq!(update.project.name, "Local draft");
    let repeated = catalog
        .sync_server(&update.project, &state(changed, true))
        .unwrap()
        .unwrap();
    assert!(repeated.conflict);
    assert_eq!(repeated.project.edit_revision, 2);
    let kept = catalog.resolve_server(&repeated.project, false).unwrap();
    assert_eq!(kept.project.edit_revision, 3);
    assert_eq!(kept.project.name, "Local draft");
}
#[test]
fn reconnect_uses_saved_base_and_ignores_older_replies() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut local = catalog
        .sync_server(&old, &state(remote(1), true))
        .unwrap()
        .unwrap()
        .project;
    local.vertical.fps = 24;
    catalog.update(local.clone());
    let mut catalog = Catalog::decode(&serde_json::to_vec(&catalog).unwrap()).unwrap();
    let mut changed = remote(4);
    changed.horizontal.fps = 25;
    let result = catalog
        .sync_server(&local, &state(changed, true))
        .unwrap()
        .unwrap();
    assert_eq!(result.project.horizontal.fps, 25);
    assert_eq!(result.project.vertical.fps, 24);
    assert!(!result.conflict);
    assert!(
        catalog
            .sync_server(&result.project, &state(remote(2), true))
            .unwrap()
            .is_none()
    );
    assert_eq!(catalog.current().edit_revision, 4);
}
#[test]
fn server_switch_preserves_drafts_and_client_accounts_are_scoped_to_workspace() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut first = catalog
        .sync_server(&old, &state(remote(1), true))
        .unwrap()
        .unwrap()
        .project;
    first
        .client_extensions
        .entry("fixture".into())
        .or_default()
        .enabled = true;
    first.name = "Unsent draft".into();
    let second = Workspace {
        id: "second-fixture".into(),
        edit_revision: 2,
        ..Default::default()
    };
    let second = catalog
        .sync_server(&first, &state(second, true))
        .unwrap()
        .unwrap()
        .project;
    assert!(second.client_extensions.is_empty());
    let back = catalog
        .sync_server(&second, &state(remote(3), true))
        .unwrap()
        .unwrap()
        .project;
    assert_eq!(back.name, "Unsent draft");
    assert!(back.client_extensions["fixture"].enabled);
}
#[test]
fn redacted_projection_never_becomes_an_editable_merge_base() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut redacted = remote(1);
    redacted.add_source();
    redacted.sources[0].key.clear();
    redacted.sources[0].url.clear();
    let mut local = catalog
        .sync_server(&old, &state(redacted.clone(), false))
        .unwrap()
        .unwrap()
        .project;
    local.name = "Read-only edit".into();
    let next = catalog
        .sync_server(&local, &state(redacted.clone(), false))
        .unwrap()
        .unwrap();
    assert_eq!(next.project.name, redacted.name);
    let mut full = redacted;
    full.sources[0].key = "fixture-private".into();
    full.sources[0].url = "rtmp://localhost:1935/live".into();
    let editable = catalog
        .sync_server(&next.project, &state(full, true))
        .unwrap()
        .unwrap();
    assert_eq!(editable.project.sources[0].key, "fixture-private");
    assert!(!editable.conflict);
}
#[test]
fn successful_apply_acknowledges_its_body_without_discarding_newer_drafts() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut sent = catalog
        .sync_server(&old, &state(remote(1), true))
        .unwrap()
        .unwrap()
        .project;
    sent.add_source();
    catalog.update(sent.clone());
    assert!(catalog.applied_server(&sent, 2, "fixture"));
    let mut local = sent.clone();
    local.sources[0].name = "Typed while applying".into();
    sent.edit_revision = 2;
    let next = catalog
        .sync_server(&local, &state(sent, true))
        .unwrap()
        .unwrap();
    assert!(!next.conflict);
    assert_eq!(next.project.sources[0].name, "Typed while applying");
    assert_eq!(next.project.edit_revision, 2);
}

#[test]
fn concurrent_source_removal_and_scene_creation_require_resolution() {
    let mut catalog = Catalog::default();
    let old = catalog.current().clone();
    let mut base = remote(1);
    base.add_source();
    let mut local = catalog
        .sync_server(&old, &state(base.clone(), true))
        .unwrap()
        .unwrap()
        .project;
    local.sources.clear();
    assert!(local.validate().is_ok());
    let mut changed = base;
    changed.edit_revision = 2;
    changed.add_scene(acheflow_media_api::workspace::Orientation::Horizontal);
    changed.scenes[0]
        .layers
        .push(acheflow_media_api::workspace::Layer {
            source_id: 1,
            width: 640,
            height: 360,
            ..Default::default()
        });
    assert!(changed.validate().is_ok());
    let update = catalog
        .sync_server(&local, &state(changed, true))
        .unwrap()
        .unwrap();
    assert!(update.conflict);
    assert!(update.project.validate().is_ok());
    assert!(update.project.sources.is_empty());
    assert!(update.project.scenes.is_empty());
    let authoritative = catalog.resolve_server(&update.project, true).unwrap();
    assert_eq!(authoritative.project.sources.len(), 1);
    assert_eq!(authoritative.project.scenes.len(), 1);
}
