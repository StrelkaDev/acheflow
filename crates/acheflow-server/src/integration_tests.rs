//! Independent, real WASM guests prove host extensibility without service branches.
use crate::control::{ControlHost, now_ms};
use acheflow_extension_api::integration::{GuestManifest, Interaction, SceneAction};
use acheflow_media_api::control::{ControlAction, ControlRequest, ControlSnapshot};
use serde_json::json;
use std::{sync::Arc, time::Duration};
fn host(ids: &[&str]) -> (Arc<ControlHost>, acheflow_media_api::workspace::Workspace) {
    let (mut host, mut workspace) = crate::control::tests::host();
    workspace.migrate_spatial();
    *host.runtime.workspace.write().unwrap() = Some(workspace.clone());
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.clear();
    for id in ids {
        let manifest:GuestManifest=serde_json::from_value(json!({"id":id,"name":id,"api_version":2,"module":"fixture.wasm","capabilities":["channel","chat","delivery","poll","donation","interaction"],"services":[],"subscriptions":["chat.send","poll.create","poll.close"]})).unwrap();
        manifest.validate().unwrap();
        inner.installed.insert(
            id.to_string(),
            crate::extension_registry::Installed {
                manifest,
                module: include_bytes!("../tests/fixtures/domain.wasm").to_vec(),
            },
        );
    }
    *inner.services.lock().unwrap() = Default::default();
    host.snapshot(0, 1);
    (host, workspace)
}
fn request(host: &ControlHost, action: ControlAction) -> ControlRequest {
    let snapshot = host.snapshot(0, 200);
    ControlRequest {
        workspace_id: snapshot.workspace_id.unwrap(),
        request_id: format!(
            "{}:{}",
            now_ms(),
            acheflow_media_api::workspace::new_token()
        ),
        expected_revision: snapshot.revision,
        action,
    }
}
async fn execute(host: &Arc<ControlHost>, action: ControlAction) -> ControlSnapshot {
    host.execute(request(host, action)).await.unwrap()
}
async fn wait(host: &ControlHost, condition: impl Fn(&ControlSnapshot) -> bool) -> ControlSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = host.snapshot(0, 200);
            if condition(&snapshot) {
                break snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("guest did not publish expected state")
}
async fn enable(host: &Arc<ControlHost>, ids: &[&str]) {
    for id in ids {
        execute(
            host,
            ControlAction::SetExtension {
                id: (*id).into(),
                enabled: true,
                config: None,
            },
        )
        .await;
    }
    wait(host, |s| s.channels.len() == ids.len()).await;
}
#[tokio::test]
async fn all_and_selected_chat_targets_merge_receipts_and_echoes_without_replaying_intents() {
    let (host, _) = host(&["alpha", "beta"]);
    enable(&host, &["alpha", "beta"]).await;
    let intent = request(
        &host,
        ControlAction::SendChat {
            targets: vec![],
            text: "all channels".into(),
        },
    );
    host.execute(intent.clone()).await.unwrap();
    let state = wait(&host, |s| {
        s.chat.entries.iter().any(|e| {
            e.message.text == "all channels"
                && e.delivery.len() == 2
                && e.delivery.iter().all(|d| d.state == "sent")
        })
    })
    .await;
    assert_eq!(
        state
            .chat
            .entries
            .iter()
            .filter(|e| e.message.text == "all channels")
            .count(),
        1
    );
    host.execute(intent).await.unwrap();
    execute(
        &host,
        ControlAction::SendChat {
            targets: vec!["beta:channel".into()],
            text: "only beta".into(),
        },
    )
    .await;
    let state = wait(&host, |s| {
        s.chat
            .entries
            .iter()
            .any(|e| e.message.text == "only beta" && e.delivery.iter().all(|d| d.state == "sent"))
    })
    .await;
    assert_eq!(
        state
            .chat
            .entries
            .iter()
            .find(|e| e.message.text == "only beta")
            .unwrap()
            .delivery
            .len(),
        1
    );
    assert!(
        host.execute(request(
            &host,
            ControlAction::SendChat {
                targets: vec!["missing:channel".into()],
                text: "invalid".into()
            }
        ))
        .await
        .is_err()
    );
}
#[tokio::test]
async fn native_polls_create_refresh_and_close_across_two_independent_guests() {
    let (host, _) = host(&["alpha", "beta"]);
    enable(&host, &["alpha", "beta"]).await;
    execute(
        &host,
        ControlAction::CreatePoll {
            poll: acheflow_extension_api::interaction::PollRequest {
                question: "Choose".into(),
                options: vec!["A".into(), "B".into()],
                channels: vec![],
                duration_seconds: 60,
            },
        },
    )
    .await;
    let state = wait(&host, |s| {
        s.polls.first().is_some_and(|p| p.votes == [6, 6])
    })
    .await;
    let poll = &state.polls[0];
    assert_eq!(poll.targets.len(), 2);
    assert!(!poll.closed);
    let id = poll.id;
    execute(&host, ControlAction::ClosePoll { id, cancel: false }).await;
    let state = wait(&host, |s| s.polls[0].closed).await;
    assert_eq!(state.polls[0].votes, [8, 8]);
    assert!(
        state.polls[0]
            .targets
            .iter()
            .all(|t| t.state == "completed")
    );
}
#[tokio::test]
async fn disabled_instances_cannot_publish_and_other_channels_keep_working() {
    let (host, _) = host(&["alpha", "beta"]);
    enable(&host, &["alpha", "beta"]).await;
    let instance = host.services.lock().unwrap().instances["alpha"].clone();
    let owner = host.snapshot(0, 1).workspace_id.unwrap();
    execute(
        &host,
        ControlAction::SetExtension {
            id: "alpha".into(),
            enabled: false,
            config: None,
        },
    )
    .await;
    assert_eq!(host.snapshot(0, 1).channels.len(), 1);
    assert!(
        host.publish_from(
            &owner,
            "alpha",
            &instance,
            json!({"type":"donation","donation":{}}),
            0
        )
        .await
        .is_err()
    );
    execute(
        &host,
        ControlAction::SetExtension {
            id: "alpha".into(),
            enabled: true,
            config: None,
        },
    )
    .await;
    wait(&host, |s| s.channels.len() == 2).await;
    assert!(
        host.publish_from(
            &owner,
            "alpha",
            &instance,
            json!({"type":"channel","channel":{"id":"channel","label":"Late old instance","capabilities":["chat.send"]}}),
            0
        )
        .await
        .is_err()
    );
    let state = wait(&host, |s| {
        s.donations.len() == 2 && s.interactions.len() == 2
    })
    .await;
    assert_eq!(
        state.donations.len(),
        2,
        "restart must deduplicate provider receipts"
    );
}
#[tokio::test]
async fn per_target_failures_are_visible_and_direct_interactions_use_the_same_scene_api() {
    let (host, workspace) = host(&["alpha", "broken"]);
    enable(&host, &["alpha", "broken"]).await;
    execute(
        &host,
        ControlAction::SendChat {
            targets: vec![],
            text: "delivery".into(),
        },
    )
    .await;
    let state = wait(&host, |s| {
        s.chat.entries.iter().any(|e| {
            e.message.platform == "acheflow" && e.delivery.iter().all(|d| d.state != "pending")
        })
    })
    .await;
    let entry = state
        .chat
        .entries
        .iter()
        .find(|e| e.message.platform == "acheflow")
        .unwrap();
    assert!(entry.delivery.iter().any(|d| d.state == "sent"));
    assert!(
        entry
            .delivery
            .iter()
            .any(|d| d.state == "failed" && d.error.is_some())
    );
    let scene = &workspace.scenes[0];
    let object = scene.spatial.as_ref().unwrap().objects[0].id;
    execute(
        &host,
        ControlAction::SetInteraction {
            interaction: Interaction {
                id: "spin".into(),
                label: "Spin".into(),
                action: Some(SceneAction {
                    scene_id: scene.id,
                    object_id: object,
                    rotation: [0., 45., 0.],
                }),
                enabled: true,
            },
        },
    )
    .await;
    execute(
        &host,
        ControlAction::TriggerInteraction {
            id: "acheflow:spin".into(),
        },
    )
    .await;
    assert_eq!(
        host.snapshot(0, 1).scenes[0]
            .spatial
            .as_ref()
            .unwrap()
            .objects[0]
            .transform
            .rotation,
        [0., 45., 0.]
    );
}
#[test]
fn production_host_and_ui_do_not_reference_installed_provider_protocols() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut forbidden = vec![];
    for entry in std::fs::read_dir(root.join("extensions"))
        .unwrap()
        .flatten()
    {
        let path = entry.path().join("manifest.json");
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let manifest: GuestManifest = serde_json::from_slice(&bytes).unwrap();
        if !manifest.services.is_empty() {
            forbidden.push(format!("\"{}\"", manifest.id));
            forbidden.extend(manifest.services);
        }
    }
    for package in ["acheflow-server", "acheflow-ui"] {
        for entry in std::fs::read_dir(root.join("crates").join(package).join("src"))
            .unwrap()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs")
                || path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().ends_with("tests"))
            {
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap();
            let production = content.split("#[cfg(test)]").next().unwrap();
            for marker in &forbidden {
                assert!(
                    !production.contains(marker),
                    "{} contains provider-specific {marker}",
                    path.display()
                );
            }
        }
    }
}

#[tokio::test]
async fn server_rejects_provider_transport_before_starting_a_guest() {
    let (mut host, workspace) = host(&["network"]);
    let extension = Arc::get_mut(&mut host)
        .unwrap()
        .installed
        .get_mut("network")
        .unwrap();
    extension.manifest.capabilities.push("http".into());
    let result =
        crate::extension_host::set_enabled(host.clone(), &workspace.id, "network", true).await;
    assert!(
        result
            .unwrap_err()
            .contains("cannot request provider transport")
    );
    assert!(host.guests.lock().await.is_empty());
    assert!(host.services.lock().unwrap().instances.is_empty());
}

#[tokio::test]
async fn client_provider_discovery_never_loads_or_starts_a_server_guest() {
    let (host, workspace) = crate::control::tests::host();
    for id in ["twitch", "youtube", "vklive"] {
        assert!(host.installed[id].module.is_empty());
        execute(
            &host,
            ControlAction::SetExtension {
                id: id.into(),
                enabled: true,
                config: None,
            },
        )
        .await;
    }
    let state = host.snapshot(0, 1);
    assert_eq!(
        state
            .extensions
            .iter()
            .filter(|e| e.state == "waiting_for_client")
            .count(),
        3
    );
    assert!(host.guests.lock().await.is_empty());
    assert!(state.panels.is_empty());
    assert!(
        host.execute(ControlRequest {
            workspace_id: workspace.id,
            request_id: format!("{}:connect", now_ms()),
            expected_revision: state.revision,
            action: ControlAction::ExtensionEvent {
                id: "twitch".into(),
                action_id: "connect".into(),
                value: String::new(),
                config: None
            }
        })
        .await
        .unwrap_err()
        .contains("not running")
    );
    assert!(serde_json::from_value::<ControlAction>(json!({"type":"set_extension_secret","id":"twitch","key":"access_token","value":"never-on-server"})).is_err());
}

#[tokio::test]
async fn domain_state_survives_snapshots_without_any_installed_extension() {
    let (host, _) = host(&[]);
    execute(
        &host,
        ControlAction::SetInteraction {
            interaction: Interaction {
                id: "local".into(),
                label: "Local action".into(),
                action: None,
                enabled: true,
            },
        },
    )
    .await;
    assert_eq!(host.snapshot(0, 1).interactions.len(), 1);
    assert_eq!(host.snapshot(0, 1).interactions.len(), 1);
}

#[tokio::test]
async fn duplicate_donations_and_interaction_triggers_are_not_rebroadcast() {
    let (host, _) = host(&["alpha"]);
    enable(&host, &["alpha"]).await;
    wait(&host, |s| s.interactions.len() == 1).await;
    let owner = host.snapshot(0, 1).workspace_id.unwrap();
    let instance = host.services.lock().unwrap().instances["alpha"].clone();
    let mut events = host.events.subscribe();
    let donation = json!({"type":"donation","donation":{"id":"new-receipt","channel_id":"channel","display_name":"Viewer","amount_minor":1,"currency":"EUR","message":"","occurred_at_ms":now_ms()}});
    let trigger =
        json!({"type":"interaction","action":"trigger","id":"wave","event_id":"unique-event"});
    for effect in [&donation, &donation, &trigger, &trigger] {
        host.publish_from(&owner, "alpha", &instance, effect.clone(), 1)
            .await
            .unwrap();
    }
    let mut donations = 0;
    let mut triggers = 0;
    while let Ok(event) = events.try_recv() {
        if event["name"] == "donation.received" && event["event"]["donation"]["id"] == "new-receipt"
        {
            donations += 1;
        }
        if event["name"] == "interaction.triggered" && event["event"]["event_id"] == "unique-event"
        {
            triggers += 1;
        }
    }
    assert_eq!(donations, 1);
    assert_eq!(triggers, 1);
}
#[tokio::test]
async fn subscribed_guest_event_cycles_stop_at_the_causal_depth_limit() {
    let (mut host, _) = host(&["alpha", "beta"]);
    for extension in Arc::get_mut(&mut host).unwrap().installed.values_mut() {
        extension.manifest.capabilities.push("read.chat".into());
        extension.manifest.subscriptions.push("chat.message".into());
        extension
            .manifest
            .settings
            .push(acheflow_extension_api::integration::Setting {
                id: "loop".into(),
                label: "Loop fixture".into(),
                ..Default::default()
            });
    }
    for id in ["alpha", "beta"] {
        execute(
            &host,
            ControlAction::SetExtension {
                id: id.into(),
                enabled: true,
                config: Some(std::collections::BTreeMap::from([(
                    "loop".into(),
                    "true".into(),
                )])),
            },
        )
        .await;
    }
    let state = wait(&host, |s| {
        s.extensions.iter().any(|e| {
            e.error
                .as_ref()
                .is_some_and(|error| error.contains("chain limit"))
        })
    })
    .await;
    assert!(state.chat.entries.len() <= 12);
    assert!(host.services.lock().unwrap().instances.len() < 2);
}

#[tokio::test]
async fn published_ownership_and_registered_ids_match_normalized_snapshots() {
    let (host, _) = host(&["alpha"]);
    enable(&host, &["alpha"]).await;
    let owner = host.snapshot(0, 1).workspace_id.unwrap();
    let instance = host.services.lock().unwrap().instances["alpha"].clone();
    let mut events = host.events.subscribe();
    host.publish_from(&owner,"alpha",&instance,json!({"type":"channel","channel":{"id":"other","extension_id":"forged","label":"Other","capabilities":["chat.send"]}}),1).await.unwrap();
    host.publish_from(&owner,"alpha",&instance,json!({"type":"donation","donation":{"id":"normalized","extension_id":"forged","channel_id":"other","display_name":"Viewer","amount_minor":1,"currency":"EUR","message":"","occurred_at_ms":now_ms()}}),1).await.unwrap();
    host.publish_from(&owner,"alpha",&instance,json!({"type":"interaction","action":"register","interaction":{"id":"registered","label":"Registered","enabled":true}}),1).await.unwrap();
    let mut channel = false;
    let mut donation = false;
    let mut interaction = false;
    while let Ok(event) = events.try_recv() {
        if event["name"] == "channel.updated" && event["event"]["channel"]["id"] == "other" {
            assert_eq!(event["event"]["channel"]["extension_id"], "alpha");
            channel = true;
        }
        if event["name"] == "donation.received" && event["event"]["donation"]["id"] == "normalized"
        {
            assert_eq!(event["event"]["donation"]["extension_id"], "alpha");
            donation = true;
        }
        if event["name"] == "interaction.registered"
            && event["event"]["interaction"]["id"] == "alpha:registered"
        {
            interaction = true;
        }
    }
    assert!(channel && donation && interaction);
    let snapshot = host.snapshot(0, 1);
    assert!(snapshot.channels.iter().any(|c| c.key() == "alpha:other"));
    assert!(
        snapshot
            .interactions
            .iter()
            .any(|i| i.id == "alpha:registered")
    );
}

#[tokio::test]
async fn disabled_interactions_cannot_be_triggered_or_reenabled_by_provider_refresh() {
    let (host, _) = host(&["alpha"]);
    enable(&host, &["alpha"]).await;
    wait(&host, |s| s.interactions.len() == 1).await;
    execute(
        &host,
        ControlAction::SetInteraction {
            interaction: Interaction {
                id: "alpha:wave".into(),
                label: "Wave".into(),
                action: None,
                enabled: false,
            },
        },
    )
    .await;
    assert!(
        host.execute(request(
            &host,
            ControlAction::TriggerInteraction {
                id: "alpha:wave".into()
            }
        ))
        .await
        .unwrap_err()
        .contains("disabled")
    );
    let owner = host.snapshot(0, 1).workspace_id.unwrap();
    let instance = host.services.lock().unwrap().instances["alpha"].clone();
    host.publish_from(&owner,"alpha",&instance,json!({"type":"interaction","action":"register","interaction":{"id":"wave","label":"Wave","enabled":true}}),1).await.unwrap();
    assert!(
        !host
            .snapshot(0, 1)
            .interactions
            .iter()
            .find(|i| i.id == "alpha:wave")
            .unwrap()
            .enabled
    );
    execute(
        &host,
        ControlAction::SetInteraction {
            interaction: Interaction {
                id: "alpha:wave".into(),
                label: "Wave".into(),
                action: None,
                enabled: true,
            },
        },
    )
    .await;
    execute(
        &host,
        ControlAction::SetExtension {
            id: "alpha".into(),
            enabled: false,
            config: None,
        },
    )
    .await;
    assert!(
        host.execute(request(
            &host,
            ControlAction::TriggerInteraction {
                id: "alpha:wave".into()
            }
        ))
        .await
        .unwrap_err()
        .contains("not active")
    );
}
