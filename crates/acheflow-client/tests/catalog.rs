#![cfg(not(target_arch = "wasm32"))]
use acheflow_client::{
    catalog::ClientCatalog,
    runtime::{Identity, ProviderRuntime},
};
use acheflow_extension_api::client_execution::{Capability, Connection};
use serde_json::json;
fn root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "acheflow-client-catalog-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[tokio::test]
async fn two_local_installations_run_real_wasm_without_any_server_catalog() {
    let root = root();
    for id in ["alpha", "beta", "logic"] {
        let directory = root.join(id);
        std::fs::create_dir_all(&directory).unwrap();
        let manifest = json!({"id":id,"name":id,"api_version":2,"module":"fixture.wasm","execution_side":if id=="logic"{"server"}else{"client"},"capabilities":["channel","chat","delivery","donation","interaction"],"services":[],"subscriptions":["chat.send"]});
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("fixture.wasm"),
            include_bytes!("../../acheflow-server/tests/fixtures/domain.wasm"),
        )
        .unwrap();
    }
    let catalog = ClientCatalog::discover_at(&root);
    assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
    assert_eq!(catalog.installed.len(), 2);
    assert!(!catalog.installed.contains_key("logic"));
    for (id, installed) in catalog.installed {
        let mut runtime = ProviderRuntime::new(
            installed.manifest,
            &installed.module,
            Identity {
                workspace_id: "local-workspace".into(),
                client_id: id.clone(),
                generation: 1,
                connection: Connection {
                    id: "local".into(),
                    provider: id,
                    account_id: "bot".into(),
                    channel_id: "channel".into(),
                    capabilities: [Capability::ChatRead, Capability::ChatSend].into(),
                },
            },
            Default::default(),
        )
        .unwrap();
        assert!(
            runtime
                .turn(json!({"type":"start"}), 1000)
                .await
                .unwrap()
                .iter()
                .any(|effect| effect["type"] == "channel")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn browser_bundle_discovers_only_client_manifests_with_real_matching_wasm() {
    let catalog = ClientCatalog::bundled();
    assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
    assert!(catalog.installed.len() >= 2);
    for installed in catalog.installed.values() {
        installed.manifest.validate().unwrap();
        assert_eq!(
            installed.manifest.execution_side,
            acheflow_extension_api::integration::ExecutionSide::Client
        );
        assert!(installed.module.starts_with(b"\0asm"));
    }
}
#[test]
fn native_catalog_rejects_escaping_modules_and_oversized_manifests() {
    let root = root();
    let directory = root.join("alpha");
    std::fs::create_dir_all(&directory).unwrap();
    let manifest = json!({"id":"alpha","name":"alpha","api_version":2,"module":"../outside.wasm","execution_side":"client","capabilities":[],"services":[]});
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = ClientCatalog::discover_at(&root);
    assert!(catalog.installed.is_empty());
    assert!(!catalog.errors.is_empty());
    std::fs::write(directory.join("manifest.json"), vec![b' '; 65537]).unwrap();
    let catalog = ClientCatalog::discover_at(&root);
    assert!(catalog.installed.is_empty());
    assert!(!catalog.errors.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
