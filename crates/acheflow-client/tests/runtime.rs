use acheflow_client::runtime::{Identity, ProviderRuntime};
use acheflow_extension_api::{client_execution::*, integration::GuestManifest};
use serde_json::json;
use std::collections::BTreeMap;

fn runtime(id: &str, network: Option<String>) -> ProviderRuntime {
    let manifest:GuestManifest=serde_json::from_value(json!({"id":id,"name":id,"api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":if network.is_some(){vec!["http"]}else{vec!["channel","chat","delivery","poll","donation","interaction"]},"services":network.iter().collect::<Vec<_>>(),"subscriptions":["chat.send"]})).unwrap();
    let connection = Connection {
        id: "local".into(),
        provider: id.into(),
        account_id: "bot".into(),
        channel_id: "channel".into(),
        capabilities: [Capability::ChatSend].into(),
    };
    let config = network
        .map(|url| BTreeMap::from([("transport_url".into(), url)]))
        .unwrap_or_default();
    ProviderRuntime::new(
        manifest,
        include_bytes!("../../acheflow-server/tests/fixtures/domain.wasm"),
        Identity {
            workspace_id: "workspace".into(),
            client_id: "client".into(),
            generation: 1,
            connection,
        },
        config,
    )
    .unwrap()
}
#[tokio::test]
async fn cancelling_a_client_turn_drops_inflight_transport_without_waiting_for_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let mut runtime = runtime("network", Some(url));
    let task = tokio::spawn(async move { runtime.turn(json!({"type":"start"}), 1000).await });
    let (_connection, _) =
        tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
    task.abort();
    tokio::time::timeout(std::time::Duration::from_millis(500), task)
        .await
        .unwrap()
        .unwrap_err();
}
#[tokio::test]
async fn client_checks_workspace_generation_expiry_and_replay_before_any_write() {
    let mut runtime = runtime("alpha", None);
    let claim = ClaimedInstruction {
        lease: 1,
        instruction: Instruction {
            connection: runtime.identity().connection.clone(),
            id: "1000:send".into(),
            request_id: None,
            workspace_id: "workspace".into(),
            client_id: "client".into(),
            generation: 1,
            connection_id: "local".into(),
            action: ClientAction::SendChat {
                text: "hello".into(),
            },
            created_at_ms: 1000,
            expires_at_ms: 2000,
            state: InstructionState::InFlight,
            external_id: None,
        },
    };
    let mut wrong = claim.clone();
    wrong.instruction.workspace_id = "another".into();
    assert_eq!(
        runtime.execute(wrong, 1001).await.unwrap().0.outcome,
        ExecutionOutcome::Rejected
    );
    let (receipt, output) = runtime.execute(claim.clone(), 1001).await.unwrap();
    assert_eq!(receipt.outcome, ExecutionOutcome::Sent);
    assert!(output.iter().any(|e| e["type"] == "delivery"));
    let (again, output) = runtime.execute(claim.clone(), 1002).await.unwrap();
    assert_eq!(again, receipt);
    assert!(output.is_empty());
    let mut changed = claim.clone();
    changed.lease += 1;
    assert!(runtime.execute(changed, 1002).await.is_err());
    let mut expired = claim.clone();
    expired.instruction.id = "1000:expired".into();
    assert_eq!(
        runtime.execute(expired, 2000).await.unwrap().0.outcome,
        ExecutionOutcome::Rejected
    );
    let mut unleased = claim.clone();
    unleased.instruction.id = "1000:no-lease".into();
    unleased.lease = 0;
    assert_eq!(
        runtime.execute(unleased, 1001).await.unwrap().0.outcome,
        ExecutionOutcome::Rejected
    );
    let mut stale = claim;
    stale.instruction.id = "1000:stale".into();
    stale.instruction.generation = 2;
    assert_eq!(
        runtime.execute(stale, 1001).await.unwrap().0.outcome,
        ExecutionOutcome::Rejected
    );
}

#[tokio::test]
async fn real_provider_guests_keep_oauth_and_authorization_ui_on_the_client() {
    for id in ["twitch", "youtube", "vklive"] {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions")
            .join(id);
        let manifest: GuestManifest =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
        let module = std::fs::read(root.join(&manifest.module)).unwrap();
        let identity = Identity {
            workspace_id: "workspace".into(),
            client_id: "client".into(),
            generation: 1,
            connection: Connection {
                id: "local".into(),
                provider: id.into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: [Capability::ChatRead].into(),
            },
        };
        let mut runtime = ProviderRuntime::new(
            manifest,
            &module,
            identity,
            if id == "vklive" {
                BTreeMap::from([("channel".into(), "streamer".into())])
            } else {
                Default::default()
            },
        )
        .unwrap();
        let output = runtime.turn(json!({"type":"start"}), 1000).await.unwrap();
        assert!(
            output
                .iter()
                .any(|e| e["type"] == "status" && e["state"] == "needs_authorization"),
            "{id}: {output:?}"
        );
        assert!(
            output
                .iter()
                .all(|e| e["type"] != "http" && e["type"] != "websocket")
        );
    }
}
