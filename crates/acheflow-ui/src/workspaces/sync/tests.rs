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
fn mixer_ack_keeps_scene_selection_and_newer_fader_edits_local() {
    use acheflow_media_api::{
        audio::{BusChange, MixerEdit},
        workspace::Orientation,
    };
    let mut server = remote(1);
    let program = server.add_scene(Orientation::Horizontal);
    let preview = server.add_scene(Orientation::Horizontal);
    server.set_active(Orientation::Horizontal, Some(program));
    let mut catalog = Catalog::default();
    let original = catalog.current().clone();
    let mut local = catalog
        .sync_server(&original, &state(server.clone(), true))
        .unwrap()
        .unwrap()
        .project;
    local.set_active(Orientation::Horizontal, Some(preview));
    local.scenes[0].audio.buses[0].gain = 122;
    local.scenes[0].name = "Unsaved scene rename".into();
    let edit = MixerEdit::Bus {
        scene: program,
        bus: 1,
        change: BusChange::Gain(113),
    };
    edit.apply(&mut server).unwrap();
    server.edit_revision = 2;
    let request = MixerRequest {
        workspace_id: local.id.clone(),
        expected_revision: 1,
        edits: vec![edit],
    };
    assert!(catalog.applied_mixer(&local, &request, MixerAck { revision: 2 }, "fixture"));
    local.edit_revision = 2;
    let update = catalog
        .sync_server(&local, &state(server, true))
        .unwrap()
        .unwrap();
    assert!(!update.conflict);
    assert_eq!(
        update.project.active(Orientation::Horizontal),
        Some(preview)
    );
    assert_eq!(update.project.scenes[0].audio.buses[0].gain, 122);
    assert_eq!(update.project.scenes[0].name, "Unsaved scene rename");
}

#[test]
fn take_ack_saves_only_that_formats_scene_selection() {
    use acheflow_media_api::{live::TakeSceneRequest, workspace::Orientation};
    let mut server = remote(1);
    let first = server.add_scene(Orientation::Horizontal);
    let second = server.add_scene(Orientation::Horizontal);
    let vertical = server.add_scene(Orientation::Vertical);
    let mut catalog = Catalog::default();
    let original = catalog.current().clone();
    let mut local = catalog
        .sync_server(&original, &state(server.clone(), true))
        .unwrap()
        .unwrap()
        .project;
    assert!(catalog.can_take_scene(&local, Orientation::Horizontal, second));
    assert!(!catalog.can_take_scene(&local, Orientation::Horizontal, vertical));
    local
        .scenes
        .iter_mut()
        .find(|scene| scene.id == second)
        .unwrap()
        .name = "Local scene edits".into();
    assert!(!catalog.can_take_scene(&local, Orientation::Horizontal, second));
    let request = TakeSceneRequest {
        workspace_id: local.id.clone(),
        expected_revision: 1,
        orientation: Orientation::Horizontal,
        scene_id: second,
    };
    assert_eq!(server.active(Orientation::Horizontal), Some(first));
    assert!(catalog.applied_delta(&local, 1, 2, "fixture", |base| request.apply(base)));
    request.apply(&mut server).unwrap();
    server.edit_revision = 2;
    request.apply(&mut local).unwrap();
    local.edit_revision = 2;
    let update = catalog
        .sync_server(&local, &state(server, true))
        .unwrap()
        .unwrap();
    assert!(!update.conflict);
    assert_eq!(update.project.active(Orientation::Horizontal), Some(second));
    assert_eq!(update.project.active(Orientation::Vertical), Some(vertical));
    assert_eq!(
        update
            .project
            .scenes
            .iter()
            .find(|scene| scene.id == second)
            .unwrap()
            .name,
        "Local scene edits"
    );
}

#[test]
fn saving_source_name_does_not_acknowledge_a_draft_capture_change() {
    use acheflow_media_api::live::SourceEdit;
    let mut server = remote(1);
    server.add_source();
    let mut catalog = Catalog::default();
    let original = catalog.current().clone();
    let mut local = catalog
        .sync_server(&original, &state(server.clone(), true))
        .unwrap()
        .unwrap()
        .project;
    let saved_key = server.sources[0].key.clone();
    local.sources[0].key = "pending-configuration".into();
    let edit = SourceEdit::Name("Renamed".into());
    edit.apply(&mut local, 1).unwrap();
    edit.apply(&mut server, 1).unwrap();
    server.edit_revision = 2;
    assert!(catalog.applied_delta(&local, 1, 2, "fixture", |base| {
        edit.apply(base, 1).map(|_| ())
    }));
    assert_eq!(
        catalog.editable_base(&local).unwrap().sources[0].key,
        saved_key
    );
    local.edit_revision = 2;
    let update = catalog
        .sync_server(&local, &state(server, true))
        .unwrap()
        .unwrap();
    assert!(!update.conflict);
    assert_eq!(update.project.sources[0].name, "Renamed");
    assert_eq!(update.project.sources[0].key, "pending-configuration");
}

#[test]
fn accepting_scene_content_preserves_newer_edits_and_the_local_mixer() {
    use acheflow_media_api::{
        live::{SceneContent, SceneEdit},
        workspace::Orientation,
    };
    let mut server = remote(1);
    let scene = server.add_scene(Orientation::Horizontal);
    let mut catalog = Catalog::default();
    let original = catalog.current().clone();
    let mut local = catalog
        .sync_server(&original, &state(server.clone(), true))
        .unwrap()
        .unwrap()
        .project;
    local.scenes[0].name = "Sent scene name".into();
    let edit = SceneEdit::Content(SceneContent::of(&local.scenes[0]));
    local.scenes[0].name = "Edited after save".into();
    local.scenes[0].audio.buses[0].gain = 83;
    assert!(catalog.applied_delta(&local, 1, 2, "fixture", |base| edit.apply(base, scene)));
    edit.apply(&mut server, scene).unwrap();
    server.edit_revision = 2;
    local.edit_revision = 2;
    let update = catalog
        .sync_server(&local, &state(server, true))
        .unwrap()
        .unwrap();
    assert!(!update.conflict);
    assert_eq!(update.project.scenes[0].name, "Edited after save");
    assert_eq!(update.project.scenes[0].audio.buses[0].gain, 83);
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
