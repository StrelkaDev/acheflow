use acheflow_control::client_execution::ExecutionRegistry;
use acheflow_extension_api::client_execution::*;
use std::collections::BTreeSet;

fn capabilities(values: &[Capability]) -> BTreeSet<Capability> {
    values.iter().copied().collect()
}
fn connection(provider: &str) -> Connection {
    Connection {
        id: "connection".into(),
        provider: provider.into(),
        account_id: "sender".into(),
        channel_id: "channel".into(),
        capabilities: capabilities(&[Capability::ChatRead, Capability::ChatSend]),
    }
}
fn policy(provider: &str) -> CapabilityGrant {
    CapabilityGrant {
        provider: provider.into(),
        account_id: "sender".into(),
        channel_id: "channel".into(),
        capabilities: capabilities(&[Capability::ChatRead, Capability::ChatSend]),
    }
}
fn registration(provider: &str, generation: u64) -> ClientRegistration {
    ClientRegistration {
        version: CLIENT_EXECUTION_VERSION,
        generation,
        connections: vec![connection(provider)],
    }
}
fn setup() -> ExecutionRegistry {
    let mut registry = ExecutionRegistry::default();
    registry
        .set_policy("workspace", vec![policy("alpha"), policy("beta")], 1000)
        .unwrap();
    registry
        .register("workspace", "client-a", registration("alpha", 1), 1000)
        .unwrap();
    registry
        .register("workspace", "client-b", registration("beta", 1), 1000)
        .unwrap();
    registry
}
fn intent(id: &str, client: &str, generation: u64, text: &str, expires: u64) -> InstructionRequest {
    InstructionRequest {
        request_id: None,
        id: id.into(),
        client_id: client.into(),
        generation,
        connection_id: "connection".into(),
        action: ClientAction::SendChat { text: text.into() },
        expires_at_ms: expires,
    }
}

#[test]
fn registration_intersects_saved_policy_and_cannot_smuggle_secret_fields() {
    let mut registry = ExecutionRegistry::default();
    assert!(
        registry
            .register("workspace", "client-a", registration("alpha", 1), 1000)
            .is_err()
    );
    let mut grant = policy("alpha");
    grant.capabilities = capabilities(&[Capability::ChatRead]);
    registry.set_policy("workspace", vec![grant], 1000).unwrap();
    let registered = registry
        .register("workspace", "client-a", registration("alpha", 1), 1000)
        .unwrap();
    assert_eq!(
        registered.connections[0].capabilities,
        capabilities(&[Capability::ChatRead])
    );
    assert!(
        registry
            .enqueue(
                "workspace",
                intent("1000:send", "client-a", 1, "hello", 5000),
                1000
            )
            .is_err()
    );
    let mut value = serde_json::to_value(registration("alpha", 1)).unwrap();
    value["access_token"] = serde_json::json!("must-not-be-accepted");
    assert!(serde_json::from_value::<ClientRegistration>(value).is_err());
}

#[test]
fn two_providers_have_separate_executors_and_receipts_are_idempotent() {
    let mut registry = setup();
    for (id, client) in [("1000:a", "client-a"), ("1000:b", "client-b")] {
        registry
            .enqueue("workspace", intent(id, client, 1, "hello", 5000), 1000)
            .unwrap();
    }
    let a = registry
        .claim("workspace", "client-a", 1, 10, 1001)
        .unwrap();
    let b = registry
        .claim("workspace", "client-b", 1, 10, 1001)
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].instruction.id, "1000:a");
    assert_eq!(b[0].instruction.id, "1000:b");
    assert!(
        registry
            .claim("workspace", "client-a", 1, 10, 1002)
            .unwrap()
            .is_empty()
    );
    let receipt = ExecutionReceipt {
        instruction_id: "1000:a".into(),
        lease: a[0].lease,
        outcome: ExecutionOutcome::Sent,
        external_id: Some("external-a".into()),
    };
    assert!(
        registry
            .report("workspace", "client-b", 1, receipt.clone(), 1002)
            .is_err()
    );
    registry
        .report("workspace", "client-a", 1, receipt.clone(), 1002)
        .unwrap();
    registry
        .report("workspace", "client-a", 1, receipt.clone(), 1003)
        .unwrap();
    let state = registry
        .enqueue(
            "workspace",
            intent("1000:a", "client-a", 1, "hello", 5000),
            1003,
        )
        .unwrap();
    assert_eq!(state.state, InstructionState::Sent);
    assert!(
        registry
            .enqueue(
                "workspace",
                intent("1000:a", "client-a", 1, "different", 5000),
                1003
            )
            .is_err()
    );
    let mut changed = receipt;
    changed.outcome = ExecutionOutcome::Failed;
    assert!(
        registry
            .report("workspace", "client-a", 1, changed, 1004)
            .is_err()
    );
}

#[test]
fn disconnect_and_new_generations_cancel_queued_work_without_retrying_uncertain_writes() {
    let mut registry = setup();
    registry
        .enqueue(
            "workspace",
            intent("1000:claimed", "client-a", 1, "first", 10000),
            1000,
        )
        .unwrap();
    let claimed = registry.claim("workspace", "client-a", 1, 1, 1001).unwrap();
    registry
        .enqueue(
            "workspace",
            intent("1000:queued", "client-a", 1, "second", 10000),
            1002,
        )
        .unwrap();
    registry
        .unregister("workspace", "client-a", 1, 1003)
        .unwrap();
    assert_eq!(
        registry
            .instruction("workspace", "1000:claimed", 1003)
            .unwrap()
            .state,
        InstructionState::Unknown
    );
    assert_eq!(
        registry
            .instruction("workspace", "1000:queued", 1003)
            .unwrap()
            .state,
        InstructionState::Cancelled
    );
    assert!(
        registry
            .register("workspace", "client-a", registration("alpha", 1), 1004)
            .is_err()
    );
    registry
        .register("workspace", "client-a", registration("alpha", 2), 1004)
        .unwrap();
    assert!(
        registry
            .claim("workspace", "client-a", 2, 10, 1005)
            .unwrap()
            .is_empty()
    );
    let receipt = ExecutionReceipt {
        instruction_id: "1000:claimed".into(),
        lease: claimed[0].lease,
        outcome: ExecutionOutcome::Sent,
        external_id: Some("late".into()),
    };
    assert!(
        registry
            .report("workspace", "client-a", 1, receipt, 1005)
            .is_err()
    );
}

#[test]
fn expiry_policy_revocation_and_workspace_isolation_remove_live_authority() {
    let mut registry = setup();
    registry
        .enqueue(
            "workspace",
            intent("1000:expires", "client-a", 1, "hello", 1500),
            1000,
        )
        .unwrap();
    assert!(
        registry
            .claim("other-workspace", "client-a", 1, 10, 1100)
            .is_err()
    );
    assert!(
        registry
            .instruction("other-workspace", "1000:expires", 1100)
            .is_none()
    );
    assert!(
        registry
            .claim("workspace", "client-a", 1, 10, 1500)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        registry
            .instruction("workspace", "1000:expires", 1500)
            .unwrap()
            .state,
        InstructionState::Expired
    );
    registry
        .enqueue(
            "workspace",
            intent("1500:revoked", "client-a", 1, "hello", 2000),
            1500,
        )
        .unwrap();
    registry
        .set_policy("workspace", vec![policy("beta")], 1501)
        .unwrap();
    assert_eq!(
        registry
            .instruction("workspace", "1500:revoked", 1501)
            .unwrap()
            .state,
        InstructionState::Cancelled
    );
    assert!(
        registry
            .claim("workspace", "client-a", 1, 10, 1502)
            .is_err()
    );
    assert!(
        registry
            .connections("workspace", 1000 + REGISTRATION_TTL_MS)
            .is_empty()
    );
}

#[test]
fn heartbeat_does_not_revive_an_expired_generation_and_uncertain_writes_stay_final() {
    let mut registry = setup();
    registry
        .register("workspace", "client-a", registration("alpha", 1), 2000)
        .unwrap();
    assert!(
        registry
            .connections("workspace", 31000)
            .iter()
            .any(|c| c.client_id == "client-a")
    );
    assert!(
        registry
            .register("workspace", "client-a", registration("alpha", 1), 32000)
            .is_err()
    );
    registry
        .register("workspace", "client-a", registration("alpha", 2), 32000)
        .unwrap();
    registry
        .enqueue(
            "workspace",
            intent("32000:uncertain", "client-a", 2, "hello", 33000),
            32000,
        )
        .unwrap();
    let claimed = registry
        .claim("workspace", "client-a", 2, 1, 32001)
        .unwrap();
    assert!(
        registry
            .claim("workspace", "client-a", 2, 1, 33000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        registry
            .instruction("workspace", "32000:uncertain", 33000)
            .unwrap()
            .state,
        InstructionState::Unknown
    );
    let receipt = ExecutionReceipt {
        instruction_id: "32000:uncertain".into(),
        lease: claimed[0].lease,
        outcome: ExecutionOutcome::Sent,
        external_id: None,
    };
    assert!(
        registry
            .report("workspace", "client-a", 2, receipt, 33001)
            .is_err()
    );
}

#[test]
fn retained_intents_are_bounded_and_old_ids_cannot_be_replayed_after_pruning() {
    let mut registry = setup();
    for n in 0..512 {
        registry
            .enqueue(
                "workspace",
                intent(&format!("1000:{n}"), "client-a", 1, "hello", 5000),
                1000,
            )
            .unwrap();
    }
    assert!(
        registry
            .enqueue(
                "workspace",
                intent("1000:overflow", "client-a", 1, "hello", 5000),
                1000
            )
            .is_err()
    );
    let now = 1000 + INTENT_RETENTION_MS;
    registry
        .register("workspace", "client-a", registration("alpha", 2), now)
        .unwrap();
    assert!(
        registry
            .enqueue(
                "workspace",
                intent("1000:0", "client-a", 2, "hello", now + 1000),
                now
            )
            .is_err()
    );
    registry
        .enqueue(
            "workspace",
            intent(&format!("{now}:new"), "client-a", 2, "hello", now + 1000),
            now,
        )
        .unwrap();
}

#[test]
fn instruction_claims_follow_enqueue_order_rather_than_lexical_intent_ids() {
    let mut registry = setup();
    registry
        .enqueue(
            "workspace",
            intent("1000:z-first", "client-a", 1, "first", 5000),
            1000,
        )
        .unwrap();
    registry
        .enqueue(
            "workspace",
            intent("1000:a-second", "client-a", 1, "second", 5000),
            1000,
        )
        .unwrap();
    let claimed = registry.claim("workspace", "client-a", 1, 2, 1001).unwrap();
    assert_eq!(claimed[0].instruction.id, "1000:z-first");
    assert_eq!(claimed[1].instruction.id, "1000:a-second");
}

#[test]
fn credential_free_server_manifests_reject_network_and_secret_permissions() {
    use acheflow_extension_api::integration::{ExecutionSide, GuestManifest};
    let value = serde_json::json!({"id":"logic","name":"Logic","api_version":2,"module":"logic.wasm","execution_side":"server","capabilities":["panel"],"services":[]});
    let manifest: GuestManifest = serde_json::from_value(value.clone()).unwrap();
    manifest.validate().unwrap();
    assert_eq!(manifest.execution_side, ExecutionSide::Server);
    for extra in [
        serde_json::json!({"capabilities":["panel","http"]}),
        serde_json::json!({"services":["https://example.org"]}),
        serde_json::json!({"settings":[{"id":"access","label":"Secret","secret":true}]}),
        serde_json::json!({"secret_fields":{"access":"/access_token"}}),
    ] {
        let mut changed = value.clone();
        for (key, value) in extra.as_object().unwrap() {
            changed[key] = value.clone();
        }
        assert!(
            serde_json::from_value::<GuestManifest>(changed)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut legacy = value;
    legacy.as_object_mut().unwrap().remove("execution_side");
    assert_eq!(
        serde_json::from_value::<GuestManifest>(legacy)
            .unwrap()
            .execution_side,
        ExecutionSide::Server
    );
}

#[test]
fn malformed_registration_cannot_replace_live_authority() {
    let mut registry = setup();
    let mut requests = vec![];
    let mut version = registration("alpha", 2);
    version.version += 1;
    requests.push(version);
    let mut duplicate = registration("alpha", 2);
    duplicate.connections.push(connection("alpha"));
    requests.push(duplicate);
    let mut invalid = registration("alpha", 2);
    invalid.connections[0].account_id = "not/an/identity".into();
    requests.push(invalid);
    let mut too_many = registration("alpha", 2);
    too_many.connections = vec![connection("alpha"); 17];
    requests.push(too_many);
    for request in requests {
        assert!(
            registry
                .register("workspace", "client-a", request, 1001)
                .is_err()
        );
    }
    assert_eq!(
        registry
            .connections("workspace", 1001)
            .iter()
            .find(|c| c.client_id == "client-a")
            .unwrap()
            .generation,
        1
    );
}

#[test]
fn moderation_requires_both_workspace_permission_and_client_advertisement() {
    let mut registry = setup();
    let action = ClientAction::TimeoutUser {
        user_id: "viewer".into(),
        duration_seconds: 60,
    };
    let mut request = intent("1000:moderation", "client-a", 1, "unused", 5000);
    request.action = action;
    assert!(
        registry
            .enqueue("workspace", request.clone(), 1000)
            .is_err()
    );
    let mut grant = policy("alpha");
    grant.capabilities.insert(Capability::UserTimeout);
    registry.set_policy("workspace", vec![grant], 1001).unwrap();
    assert!(
        registry
            .enqueue("workspace", request.clone(), 1001)
            .is_err()
    );
    let mut registration = registration("alpha", 2);
    registration.connections[0]
        .capabilities
        .insert(Capability::UserTimeout);
    registry
        .register("workspace", "client-a", registration, 1002)
        .unwrap();
    request.generation = 2;
    registry.enqueue("workspace", request, 1002).unwrap();
    let mut invalid = intent("1002:invalid", "client-a", 2, "unused", 5000);
    invalid.action = ClientAction::TimeoutUser {
        user_id: "viewer".into(),
        duration_seconds: 0,
    };
    assert!(registry.enqueue("workspace", invalid, 1002).is_err());
}

#[test]
fn a_channel_cannot_have_two_collectors_or_conflicting_senders() {
    let mut registry = setup();
    assert!(
        registry
            .register(
                "workspace",
                "another-client",
                registration("alpha", 1),
                1001
            )
            .unwrap_err()
            .contains("already has an active executor")
    );
    registry
        .unregister("workspace", "client-a", 1, 1002)
        .unwrap();
    assert!(
        registry
            .register(
                "workspace",
                "another-client",
                registration("alpha", 1),
                1003
            )
            .is_ok()
    );
}
