//! Independent real-WASM source producers share the same namespace/lifecycle contract.
use acheflow_extension_api::integration::GuestManifest;
use acheflow_media_api::visual::{ExtensionSource, SourceKind};
use serde_json::json;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[tokio::test]
async fn visual_sources_are_declared_owned_and_withdrawn_with_the_instance() {
    let (mut host, workspace) = crate::control::tests::host();
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.clear();
    for (id, type_id, module) in [
        (
            "alpha",
            "actions",
            include_bytes!("../../../extensions/sample/sample.wasm").as_slice(),
        ),
        (
            "beta",
            "fixture-text",
            include_bytes!("../tests/fixtures/domain.wasm").as_slice(),
        ),
    ] {
        let manifest:GuestManifest=serde_json::from_value(json!({"id":id,"name":id,"api_version":2,"module":"source.wasm","execution_side":"server","capabilities":["panel","source"],"services":[],"source_types":[{"id":type_id,"name":type_id}]})).unwrap();
        manifest.validate().unwrap();
        inner.installed.insert(
            id.into(),
            crate::extension_registry::Installed {
                manifest,
                module: module.to_vec(),
            },
        );
        let mut current = inner.runtime.workspace.write().unwrap();
        let current = current.as_mut().unwrap();
        current.add_source();
        let source = current.sources.last_mut().unwrap();
        source.kind = SourceKind::Extension(ExtensionSource {
            extension_id: id.into(),
            type_id: type_id.into(),
            ..Default::default()
        });
        source.url.clear();
        source.key.clear();
        let config = current.extensions.entry(id.into()).or_default();
        config.enabled = true;
        if id == "beta" {
            config
                .config
                .insert("visual_source_fixture".into(), "true".into());
        }
    }
    host.snapshot(0, 1);
    for id in ["alpha", "beta"] {
        crate::extension_host::set_enabled(host.clone(), &workspace.id, id, true)
            .await
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if host.snapshot(0, 1).source_frames.len() == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "source producers did not publish"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let snapshot = host.snapshot(0, 1);
    assert_eq!(snapshot.source_types.len(), 2);
    let alpha = snapshot
        .source_frames
        .iter()
        .find(|f| f.extension_id == "alpha")
        .unwrap();
    let beta = snapshot
        .source_frames
        .iter()
        .find(|f| f.extension_id == "beta")
        .unwrap();
    let old = host.services.lock().unwrap().instances["alpha"].clone();
    let spoof = json!({"type":"source","frame":{"extension_id":"beta","source_id":beta.source_id,"type_id":beta.type_id,"document":beta.document}});
    assert!(
        host.publish_from(&workspace.id, "alpha", &old, spoof, 0)
            .await
            .is_err()
    );
    let effect = json!({"type":"source","frame":alpha});
    crate::extension_host::set_enabled(host.clone(), &workspace.id, "alpha", true)
        .await
        .unwrap();
    assert!(
        host.publish_from(&workspace.id, "alpha", &old, effect.clone(), 0)
            .await
            .is_err()
    );
    for id in ["alpha", "beta"] {
        host.runtime
            .workspace
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .extensions
            .get_mut(id)
            .unwrap()
            .enabled = false;
        crate::extension_host::set_enabled(host.clone(), &workspace.id, id, false)
            .await
            .unwrap();
    }
    assert!(host.snapshot(0, 1).source_frames.is_empty());
    assert!(
        host.publish_from(&workspace.id, "alpha", &old, effect, 0)
            .await
            .is_err()
    );
    *host.runtime.workspace.write().unwrap() =
        Some(acheflow_media_api::workspace::Workspace::default());
    assert!(host.snapshot(0, 1).source_frames.is_empty());
    host.runtime.shutdown.cancel();
}
