use super::*;
use acheflow_media_api::control::{ControlAction, ControlRequest};
fn fixture() -> (Arc<ControlHost>, Workspace) {
    let (mut host, mut workspace) = crate::control::tests::host();
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.clear();
    inner.provider_catalog.installed.clear();
    inner.providers.set_server("fixture-server").unwrap();
    for id in ["alpha", "beta"] {
        let manifest=serde_json::from_value(serde_json::json!({"id":id,"name":id,"api_version":2,"module":"fixture.wasm","execution_side":"client","services":[],"capabilities":["channel","chat","delivery","status"],"settings":[{"id":"server_fixture","label":"Fixture"},{"id":"server_auth_lost","label":"Expired"}],"subscriptions":["chat.send"]})).unwrap();
        inner.provider_catalog.installed.insert(
            id.into(),
            acheflow_client::catalog::Installed {
                manifest,
                module: include_bytes!("../../tests/fixtures/domain.wasm").to_vec(),
                browser_auth: None,
            },
        );
        workspace.server_integrations.insert(
            id.into(),
            ServerIntegration {
                enabled: true,
                application: "fixture".into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                config: BTreeMap::from([("server_fixture".into(), "true".into())]),
            },
        );
        workspace.client_capabilities.push(CapabilityGrant {
            provider: id.into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: BTreeSet::from([Capability::ChatRead, Capability::ChatSend]),
        });
    }
    *inner.runtime.workspace.write().unwrap() = Some(workspace.clone());
    (host, workspace)
}
async fn ready(host: &ControlHost) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let snapshot = host.snapshot(0, 20);
            if snapshot
                .server_providers
                .iter()
                .filter(|p| p.state == "running")
                .count()
                == 2
                && snapshot.chat.entries.len() >= 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "provider states: {:?}",
            host.snapshot(0, 20).server_providers
        )
    });
}
#[tokio::test]
async fn two_real_wasm_providers_collect_once_and_send_from_server_without_a_client() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    let actors: Vec<_> = host
        .providers
        .running
        .lock()
        .await
        .values()
        .map(|r| r.actor.clone())
        .collect();
    host.providers.reconcile(&host).await;
    assert_eq!(
        actors,
        host.providers
            .running
            .lock()
            .await
            .values()
            .map(|r| r.actor.clone())
            .collect::<Vec<_>>()
    );
    host.execute(ControlRequest {
        workspace_id: workspace.id,
        request_id: format!("{}:server-send", crate::control::now_ms()),
        expected_revision: host.snapshot(0, 1).revision,
        action: ControlAction::SendChat {
            targets: vec!["alpha:channel".into(), "beta:channel".into()],
            text: "Server message".into(),
        },
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let instructions = host.client_execution.lock().unwrap().instructions(
                &host.runtime.saved_state().1.unwrap().id,
                crate::control::now_ms(),
            );
            if instructions.len() == 2
                && instructions
                    .iter()
                    .all(|i| i.state == InstructionState::Sent)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    host.providers.suspend(&host).await;
    assert!(host.snapshot(0, 20).channels.is_empty());
}

#[tokio::test]
async fn primary_channels_are_reserved_and_replaced_instances_cannot_publish_late_events() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    let actor = host.providers.running.lock().await["alpha"].actor.clone();
    let generation = host
        .snapshot(0, 1)
        .server_providers
        .iter()
        .find(|p| p.provider == "alpha")
        .unwrap()
        .generation;
    let connection = host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace.id, crate::control::now_ms())
        .into_iter()
        .find(|s| s.client_id == actor)
        .unwrap()
        .connections[0]
        .clone();
    let external = ExecutorCredential {
        id: "external".into(),
        workspace_id: workspace.id.clone(),
        token: String::new(),
    };
    let error = crate::client_bridge::call(
        &host,
        &external,
        ExecutorRequest::Register {
            registration: ClientRegistration {
                version: 1,
                generation: 1,
                connections: vec![connection.clone()],
            },
        },
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("forbidden:"));
    host.runtime
        .update_workspace(&workspace.id, |w| {
            w.server_integrations.get_mut("alpha").unwrap().application = "replacement".into();
            Ok(())
        })
        .unwrap();
    host.providers.reconcile(&host).await;
    let old = ExecutorCredential {
        id: actor,
        workspace_id: workspace.id.clone(),
        token: String::new(),
    };
    assert!(
        crate::client_bridge::call_server(
            &host,
            "alpha",
            &old,
            ExecutorRequest::Register {
                registration: ClientRegistration {
                    version: 1,
                    generation,
                    connections: vec![connection]
                }
            }
        )
        .await
        .unwrap_err()
        .contains("ownership changed")
    );
    ready(&host).await;
    host.runtime
        .update_workspace(&workspace.id, |w| {
            w.server_integrations.get_mut("alpha").unwrap().enabled = false;
            Ok(())
        })
        .unwrap();
    host.providers.reconcile(&host).await;
    assert!(
        !host
            .snapshot(0, 20)
            .channels
            .iter()
            .any(|c| c.extension_id == "alpha")
    );
    host.providers.suspend(&host).await;
}
#[tokio::test]
async fn capability_revocation_and_workspace_switch_retire_provider_authority() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    host.execute(ControlRequest {
        workspace_id: workspace.id.clone(),
        request_id: format!("{}:revoke", crate::control::now_ms()),
        expected_revision: host.snapshot(0, 1).revision,
        action: ControlAction::SetClientCapabilities { grants: vec![] },
    })
    .await
    .unwrap();
    assert!(host.snapshot(0, 20).channels.is_empty());
    assert!(!host.providers.active().await);
    let next = Workspace {
        edit_revision: host.runtime.saved_state().0,
        ..Default::default()
    };
    host.runtime.install(next, true).await.unwrap();
    host.providers.reconcile(&host).await;
    assert!(
        host.snapshot(0, 20)
            .server_providers
            .iter()
            .all(|p| p.state == "stopped")
    );
}
#[tokio::test]
async fn replacement_never_requeues_an_uncertain_send_and_request_replay_is_idempotent() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    let request = ControlRequest {
        workspace_id: workspace.id.clone(),
        request_id: format!("{}:uncertain", crate::control::now_ms()),
        expected_revision: host.snapshot(0, 1).revision,
        action: ControlAction::SendChat {
            targets: vec!["alpha:channel".into()],
            text: "Once".into(),
        },
    };
    host.execute(request.clone()).await.unwrap();
    let command = host.command_lock.lock().await;
    let actor = host.providers.running.lock().await["alpha"].actor.clone();
    let status = host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace.id, crate::control::now_ms())
        .into_iter()
        .find(|s| s.client_id == actor)
        .unwrap();
    let claims = host
        .client_execution
        .lock()
        .unwrap()
        .claim(
            &workspace.id,
            &actor,
            status.generation,
            1,
            crate::control::now_ms(),
        )
        .unwrap();
    assert_eq!(claims.len(), 1);
    host.runtime
        .update_workspace(&workspace.id, |w| {
            w.server_integrations.get_mut("alpha").unwrap().application = "replacement".into();
            Ok(())
        })
        .unwrap();
    host.providers.reconcile(&host).await;
    let retained = host
        .client_execution
        .lock()
        .unwrap()
        .instructions(&workspace.id, crate::control::now_ms());
    assert_eq!(retained[0].state, InstructionState::Unknown);
    drop(command);
    ready(&host).await;
    host.execute(request).await.unwrap();
    assert_eq!(
        host.client_execution
            .lock()
            .unwrap()
            .instructions(&workspace.id, crate::control::now_ms())
            .len(),
        1
    );
    host.providers.suspend(&host).await;
}
#[tokio::test]
async fn server_shutdown_cancels_collectors_and_withdraws_channels() {
    let (host, _) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    host.runtime.shutdown.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while host.providers.active().await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(host.snapshot(0, 20).channels.is_empty());
    assert!(
        host.snapshot(0, 20)
            .server_providers
            .iter()
            .all(|p| p.state == "stopped")
    );
}

#[tokio::test]
async fn repeated_replacements_do_not_exhaust_executor_capacity() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    for replacement in 0..70 {
        host.runtime
            .update_workspace(&workspace.id, |w| {
                w.server_integrations.get_mut("alpha").unwrap().application =
                    format!("replacement-{replacement}");
                Ok(())
            })
            .unwrap();
        host.providers.reconcile(&host).await;
        ready(&host).await;
    }
    assert_eq!(
        host.client_execution
            .lock()
            .unwrap()
            .connections(&workspace.id, crate::control::now_ms())
            .len(),
        2
    );
    host.providers.suspend(&host).await;
}

#[tokio::test]
async fn provider_authorization_loss_withdraws_authority_without_a_retry_loop() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    host.runtime
        .update_workspace(&workspace.id, |w| {
            w.server_integrations
                .get_mut("alpha")
                .unwrap()
                .config
                .insert("server_auth_lost".into(), "true".into());
            Ok(())
        })
        .unwrap();
    host.providers.reconcile(&host).await;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if host
                .snapshot(0, 20)
                .server_providers
                .iter()
                .any(|p| p.provider == "alpha" && p.state == "needs_authorization")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !host
            .snapshot(0, 20)
            .channels
            .iter()
            .any(|c| c.extension_id == "alpha")
    );
    assert!(
        host.snapshot(0, 20)
            .channels
            .iter()
            .any(|c| c.extension_id == "beta")
    );
    assert!(
        host.providers.running.lock().await["alpha"]
            .join
            .is_finished()
    );
    host.providers.suspend(&host).await;
}

#[tokio::test]
async fn lease_maintenance_does_not_depend_on_provider_turn_completion() {
    let (host, workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    let actor = host.providers.running.lock().await["alpha"].actor.clone();
    let first = host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace.id, crate::control::now_ms())
        .into_iter()
        .find(|s| s.client_id == actor)
        .unwrap()
        .expires_at_ms;
    let bridge = reactor::Bridge {
        host: Arc::downgrade(&host),
        provider: "alpha".into(),
        actor: ExecutorCredential {
            id: actor.clone(),
            workspace_id: workspace.id.clone(),
            token: String::new(),
        },
    };
    tokio::select! {result=reactor::heartbeat(&bridge,std::time::Duration::from_millis(20))=>panic!("heartbeat stopped: {result:?}"),_=tokio::time::sleep(std::time::Duration::from_millis(90))=>{}}
    let next = host
        .client_execution
        .lock()
        .unwrap()
        .connections(&workspace.id, crate::control::now_ms())
        .into_iter()
        .find(|s| s.client_id == actor)
        .unwrap()
        .expires_at_ms;
    assert!(next > first);
    host.providers.suspend(&host).await;
    assert!(
        crate::client_bridge::call_server(
            &host,
            "alpha",
            &bridge.actor,
            ExecutorRequest::Register {
                registration: ClientRegistration {
                    version: 1,
                    generation: 2,
                    connections: vec![]
                }
            }
        )
        .await
        .is_err()
    );
}

#[test]
fn a_missing_server_package_is_visible_without_falling_back_to_a_client_account() {
    let (mut host, workspace) = fixture();
    Arc::get_mut(&mut host)
        .unwrap()
        .provider_catalog
        .installed
        .remove("alpha");
    let status = host
        .snapshot(0, 1)
        .server_providers
        .into_iter()
        .find(|p| p.provider == "alpha")
        .unwrap();
    assert!(status.server_owned);
    assert_eq!(status.state, "failed");
    assert!(validate(&host, &workspace).is_err());
}

#[tokio::test]
async fn disconnecting_one_provider_fences_it_without_restarting_the_other_collector() {
    let (host, _workspace) = fixture();
    host.providers.reconcile(&host).await;
    ready(&host).await;
    let beta = host.providers.running.lock().await["beta"].actor.clone();
    host.providers.stop_provider(&host, "alpha").await;
    let snapshot = host.snapshot(0, 20);
    assert_eq!(snapshot.client_executors.len(), 1);
    assert_eq!(snapshot.client_executors[0].client_id, beta);
    assert!(
        snapshot
            .server_providers
            .iter()
            .find(|p| p.provider == "alpha")
            .unwrap()
            .server_owned
    );
    host.providers.reconcile(&host).await;
    ready(&host).await;
    assert_eq!(host.providers.running.lock().await["beta"].actor, beta);
    host.providers.suspend(&host).await;
}
