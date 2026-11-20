use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::{ZipWriter, write::SimpleFileOptions};
struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "acheflow-package-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn manifest() -> serde_json::Value {
    json!({"id":"independent","name":"Independent","api_version":2,"parts_version":1,"parts":{
        "client":{"kind":"provider","module":"client.wasm","capabilities":["chat","channel"],"secret_fields":{"local_token":"/token"}},
        "server":{"kind":"provider","module":"server.wasm","capabilities":["chat","channel"],"secret_fields":{"server_token":"/token"}}
    }})
}
fn zip_files(files: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn fixture(value: serde_json::Value) -> Vec<u8> {
    let wasm = include_bytes!("../../../acheflow-server/tests/fixtures/domain.wasm").to_vec();
    zip_files(vec![
        ("manifest.json", serde_json::to_vec(&value).unwrap()),
        ("client.wasm", wasm.clone()),
        ("server.wasm", wasm),
    ])
}
#[test]
fn selected_host_is_reviewed_and_only_its_artifacts_and_secret_declarations_are_installed() {
    let temp = Temporary::new();
    let store = Store::open(temp.0.clone(), ExecutionHost::Server).unwrap();
    let bytes = fixture(manifest());
    let review = store.stage(&bytes, 0).unwrap();
    assert_eq!(review.host, ExecutionHost::Server);
    assert!(!review.execution_allowed);
    let prepared = store.prepare(&review.digest, None).unwrap();
    store.commit(&prepared).unwrap();
    let installed = store.installed().unwrap();
    assert_eq!(installed.len(), 1);
    assert!(!installed[0].review.execution_allowed);
    assert!(
        installed[0]
            .manifest
            .secret_fields
            .contains_key("server_token")
    );
    assert!(
        !installed[0]
            .manifest
            .secret_fields
            .contains_key("local_token")
    );
    let directory = temp.0.join("versions").join(&review.digest);
    assert!(directory.join("server.wasm").is_file());
    assert!(!directory.join("client.wasm").exists());
    let disk: GuestManifest =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert!(disk.parts.unwrap().client.is_none());
    drop(store);
    let restored = Store::open(temp.0.clone(), ExecutionHost::Server).unwrap();
    assert!(!restored.execution_allowed("independent"));
    restored
        .allow_execution("independent", &review.digest)
        .unwrap();
    assert!(restored.execution_allowed("independent"));
}
#[test]
fn traversal_links_extra_artifacts_bombs_unknown_versions_and_invalid_modules_are_rejected() {
    for name in [
        "../escape",
        "/absolute",
        "nested/../../escape",
        "bad\\path",
        "extra.env",
    ] {
        let bytes = zip_files(vec![
            ("manifest.json", serde_json::to_vec(&manifest()).unwrap()),
            (name, vec![0]),
        ]);
        assert!(
            inspect(&bytes, ExecutionHost::Server).is_err(),
            "accepted {name}"
        );
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_symlink("link", "/outside", SimpleFileOptions::default())
        .unwrap();
    assert!(inspect(&zip.finish().unwrap().into_inner(), ExecutionHost::Server).is_err());
    let bytes = zip_files(vec![
        ("manifest.json", serde_json::to_vec(&manifest()).unwrap()),
        ("server.wasm", vec![0; 4 * 1024 * 1024 + 1]),
    ]);
    assert!(inspect(&bytes, ExecutionHost::Server).is_err());
    let mut value = manifest();
    value["parts_version"] = json!(99);
    assert!(inspect(&fixture(value), ExecutionHost::Server).is_err());
    let mut value = manifest();
    value["parts"]["server"]["entry_point"] = json!("missing");
    assert!(inspect(&fixture(value), ExecutionHost::Server).is_err());
    let bytes = zip_files(vec![
        ("manifest.json", serde_json::to_vec(&manifest()).unwrap()),
        ("server.wasm", b"not-wasm".to_vec()),
    ]);
    assert!(inspect(&bytes, ExecutionHost::Server).is_err());
    assert!(inspect(&vec![0; MAX_ARCHIVE_BYTES + 1], ExecutionHost::Server).is_err());
}
#[test]
fn failed_and_stale_upgrades_preserve_the_previous_package_and_execution_consent() {
    let temp = Temporary::new();
    let store = Store::open(temp.0.clone(), ExecutionHost::Server).unwrap();
    let first = store.stage(&fixture(manifest()), 0).unwrap();
    store
        .commit(&store.prepare(&first.digest, None).unwrap())
        .unwrap();
    store.allow_execution(&first.id, &first.digest).unwrap();
    assert!(store.stage(b"invalid", 0).is_err());
    assert!(store.execution_allowed(&first.id));
    let mut value = manifest();
    value["name"] = json!("Next");
    let next = store.stage(&fixture(value), 0).unwrap();
    assert!(store.prepare(&next.digest, None).is_err());
    assert_eq!(store.installed().unwrap()[0].review.digest, first.digest);
    let prepared = store.prepare(&next.digest, Some(&first.digest)).unwrap();
    store.commit(&prepared).unwrap();
    assert!(!store.execution_allowed(&first.id));
    assert!(store.allow_execution(&first.id, &first.digest).is_err());
    assert_eq!(store.installed().unwrap()[0].review.digest, next.digest);
}
#[test]
fn absent_host_part_and_concurrent_store_owners_are_rejected() {
    let mut value = manifest();
    value["parts"].as_object_mut().unwrap().remove("server");
    assert!(inspect(&fixture(value), ExecutionHost::Server).is_err());
    let temp = Temporary::new();
    let _store = Store::open(temp.0.clone(), ExecutionHost::Server).unwrap();
    assert!(Store::open(temp.0.clone(), ExecutionHost::Server).is_err());
}

#[test]
fn two_independent_repository_providers_validate_for_both_hosts_without_credentials() {
    for id in ["youtube", "vklive"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions")
            .join(id);
        let bytes = std::fs::read(root.join("manifest.json")).unwrap();
        let package: GuestManifest = serde_json::from_slice(&bytes).unwrap();
        let mut names = BTreeSet::from(["manifest.json".to_owned()]);
        for host in [ExecutionHost::Client, ExecutionHost::Server] {
            let manifest = package.select_part(host).unwrap().unwrap();
            names.insert(manifest.module);
            if let Some(name) = manifest
                .authentication
                .and_then(|a| a["browser_module"].as_str().map(str::to_owned))
            {
                names.insert(name);
            }
        }
        let content: Vec<_> = names
            .iter()
            .map(|name| (name.as_str(), std::fs::read(root.join(name)).unwrap()))
            .collect();
        let bytes = zip_files(content);
        for host in [ExecutionHost::Client, ExecutionHost::Server] {
            let prepared = inspect(&bytes, host).unwrap();
            assert_eq!(prepared.manifest.selected_host(), Some(host));
            assert!(!prepared.review.execution_allowed);
            assert!(prepared.review.requires_authorization);
        }
    }
}
#[test]
fn lost_authority_after_validation_cannot_persist_a_staged_package() {
    let temp = Temporary::new();
    let store = Store::open(temp.0.clone(), ExecutionHost::Server).unwrap();
    assert!(
        store
            .stage_checked(&fixture(manifest()), 0, || Err("revoked".into()))
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(temp.0.join("staging")).unwrap().count(),
        0
    );
}
