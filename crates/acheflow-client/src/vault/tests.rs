use super::store::{self, Backend, LEGACY_SERVICE, WORKSPACE_SERVICE};
use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

#[derive(Default)]
struct Memory {
    records: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    fail_write: Cell<bool>,
    fail_delete: Cell<bool>,
    fail_read: Cell<bool>,
    legacy_reads: Cell<usize>,
    writes: Cell<usize>,
}
impl Backend for Memory {
    fn read(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        if service == LEGACY_SERVICE {
            self.legacy_reads.set(self.legacy_reads.get() + 1);
        }
        if self.fail_read.get() {
            return Err("test vault locked".into());
        }
        Ok(self
            .records
            .borrow()
            .get(&(service.into(), key.into()))
            .cloned())
    }
    fn write(&self, service: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.writes.set(self.writes.get() + 1);
        if self.fail_write.get() {
            return Err("test save failed".into());
        }
        self.records
            .borrow_mut()
            .insert((service.into(), key.into()), bytes.into());
        Ok(())
    }
    fn delete(&self, service: &str, key: &str) -> Result<(), String> {
        if self.fail_delete.get() {
            return Err("test cleanup failed".into());
        }
        self.records
            .borrow_mut()
            .remove(&(service.into(), key.into()));
        Ok(())
    }
}
fn state(token: &str) -> LocalState {
    LocalState {
        secrets: BTreeMap::from([("access_token".into(), token.into())]),
        storage: serde_json::json!({"account":"fixture"}),
    }
}
fn fixture_key(provider: &str, application: &str) -> String {
    key(
        "https://studio.example",
        "workspace-fixture",
        provider,
        application,
    )
    .unwrap()
}
fn assert_token(backend: &Memory, key: &str, expected: &str) {
    let restored = store::load(backend, key).unwrap().unwrap();
    assert_eq!(restored.secrets["access_token"], expected);
    assert_eq!(restored.storage["account"], "fixture");
}
fn legacy(backend: &Memory, key: &str, token: &str) {
    backend
        .write(
            LEGACY_SERVICE,
            key,
            &serde_json::to_vec(&serde_json::json!({
                "secrets":{"access_token":token}, "storage":{"account":"fixture"}
            }))
            .unwrap(),
        )
        .unwrap();
}

#[test]
fn extensions_and_applications_share_one_workspace_record_but_not_credentials() {
    let backend = Memory::default();
    let keys = [
        fixture_key("provider-a", "app-a"),
        fixture_key("provider-b", "app-a"),
        fixture_key("provider-a", "app-b"),
    ];
    for (i, key) in keys.iter().enumerate() {
        store::save(&backend, key, &state(&format!("fixture-token-{i}"))).unwrap();
    }
    assert_eq!(backend.records.borrow().len(), 1);
    assert_eq!(
        backend.records.borrow().keys().next().unwrap().0,
        WORKSPACE_SERVICE
    );
    for (i, key) in keys.iter().enumerate() {
        assert_token(&backend, key, &format!("fixture-token-{i}"));
    }
}

#[test]
fn workspace_and_server_boundaries_are_independent() {
    let backend = Memory::default();
    let first = fixture_key("provider-a", "app-a");
    let second = key(
        "https://studio.example",
        "other-workspace",
        "provider-a",
        "app-a",
    )
    .unwrap();
    let third = key(
        "https://other.example",
        "workspace-fixture",
        "provider-a",
        "app-a",
    )
    .unwrap();
    for (key, token) in [(&first, "one"), (&second, "two"), (&third, "three")] {
        store::save(&backend, key, &state(token)).unwrap();
    }
    assert_eq!(backend.records.borrow().len(), 3);
    assert_token(&backend, &first, "one");
    assert_token(&backend, &second, "two");
    assert_token(&backend, &third, "three");
}

#[test]
fn disconnect_removes_only_its_slot_and_deletes_the_last_empty_container() {
    let backend = Memory::default();
    let first = fixture_key("provider-a", "app-a");
    let second = fixture_key("provider-b", "app-a");
    store::save(&backend, &first, &state("one")).unwrap();
    store::save(&backend, &second, &state("two")).unwrap();
    store::save(&backend, &first, &LocalState::default()).unwrap();
    assert!(store::load(&backend, &first).unwrap().is_none());
    assert_token(&backend, &second, "two");
    store::remove(&backend, &second).unwrap();
    store::remove(&backend, &second).unwrap();
    assert!(backend.records.borrow().is_empty());
}

#[test]
fn old_records_migrate_without_losing_neighbor_accounts() {
    let backend = Memory::default();
    let first = fixture_key("provider-a", "app-a");
    let second = fixture_key("provider-b", "app-a");
    legacy(&backend, &first, "one");
    legacy(&backend, &second, "two");
    assert_token(&backend, &first, "one");
    assert!(backend.read(LEGACY_SERVICE, &first).unwrap().is_none());
    assert!(backend.read(LEGACY_SERVICE, &second).unwrap().is_some());
    assert_token(&backend, &second, "two");
    assert_eq!(backend.records.borrow().len(), 1);
    assert_token(&backend, &first, "one");
}

#[test]
fn failed_migration_write_keeps_the_legacy_record_recoverable() {
    let backend = Memory::default();
    let key = fixture_key("provider-a", "app-a");
    legacy(&backend, &key, "old");
    backend.fail_write.set(true);
    assert!(store::load(&backend, &key).is_err());
    assert!(backend.read(LEGACY_SERVICE, &key).unwrap().is_some());
    backend.fail_write.set(false);
    assert_token(&backend, &key, "old");
}

#[test]
fn cleanup_failure_never_restores_an_old_token_over_a_new_one() {
    let backend = Memory::default();
    let key = fixture_key("provider-a", "app-a");
    legacy(&backend, &key, "old");
    backend.fail_delete.set(true);
    assert!(store::save(&backend, &key, &state("new")).is_err());
    assert!(backend.read(LEGACY_SERVICE, &key).unwrap().is_some());
    backend.fail_delete.set(false);
    assert_token(&backend, &key, "new");
    assert!(backend.read(LEGACY_SERVICE, &key).unwrap().is_none());
}

#[test]
fn failed_disconnect_keeps_neighbor_and_retries_without_reviving_legacy_data() {
    let backend = Memory::default();
    let first = fixture_key("provider-a", "app-a");
    let second = fixture_key("provider-b", "app-a");
    store::save(&backend, &first, &state("new")).unwrap();
    store::save(&backend, &second, &state("neighbor")).unwrap();
    legacy(&backend, &first, "old");
    backend.fail_write.set(true);
    assert!(store::remove(&backend, &first).is_err());
    backend.fail_write.set(false);
    assert_token(&backend, &first, "new");
    assert_token(&backend, &second, "neighbor");
    store::remove(&backend, &first).unwrap();
    assert!(store::load(&backend, &first).unwrap().is_none());
    assert_token(&backend, &second, "neighbor");
}

#[test]
fn unreadable_or_corrupt_workspace_record_cannot_be_overwritten_or_bypassed() {
    let backend = Memory::default();
    let key = fixture_key("provider-a", "app-a");
    store::save(&backend, &key, &state("new")).unwrap();
    legacy(&backend, &key, "old");
    backend.fail_read.set(true);
    assert!(store::load(&backend, &key).is_err());
    assert!(store::save(&backend, &key, &state("replacement")).is_err());
    backend.fail_read.set(false);
    let container = backend
        .records
        .borrow()
        .keys()
        .find(|(service, _)| service == WORKSPACE_SERVICE)
        .unwrap()
        .clone();
    backend
        .records
        .borrow_mut()
        .insert(container.clone(), b"corrupt".to_vec());
    assert!(store::load(&backend, &key).is_err());
    assert!(store::save(&backend, &key, &state("replacement")).is_err());
    assert!(store::remove(&backend, &key).is_err());
    assert_eq!(backend.records.borrow()[&container], b"corrupt");
    assert!(backend.read(LEGACY_SERVICE, &key).unwrap().is_some());
}

#[test]
fn malformed_or_oversized_input_never_changes_existing_credentials() {
    let backend = Memory::default();
    let key = fixture_key("provider-a", "app-a");
    store::save(&backend, &key, &state("original")).unwrap();
    assert!(store::save(&backend, "invalid-identity", &state("new")).is_err());
    let mut large = state("large");
    large.storage = serde_json::Value::String("x".repeat(256 * 1024));
    assert!(store::save(&backend, &key, &large).is_err());
    assert_token(&backend, &key, "original");
}

#[test]
fn workspace_account_limit_allows_updates_and_disconnect_but_not_new_slots() {
    let backend = Memory::default();
    for i in 0..64 {
        store::save(
            &backend,
            &fixture_key("provider-a", &format!("app-{i}")),
            &state("fixture"),
        )
        .unwrap();
    }
    assert!(
        store::save(
            &backend,
            &fixture_key("provider-b", "extra"),
            &state("fixture")
        )
        .is_err()
    );
    let first = fixture_key("provider-a", "app-0");
    store::save(&backend, &first, &state("updated")).unwrap();
    assert_token(&backend, &first, "updated");
    store::remove(&backend, &first).unwrap();
    store::save(
        &backend,
        &fixture_key("provider-b", "extra"),
        &state("replacement"),
    )
    .unwrap();
    assert_token(&backend, &fixture_key("provider-a", "app-63"), "fixture");
}

#[test]
fn workspace_size_limit_preserves_previously_saved_accounts() {
    let backend = Memory::default();
    let mut large = state("fixture");
    large.storage = serde_json::Value::String("x".repeat(250 * 1024));
    for i in 0..16 {
        store::save(
            &backend,
            &fixture_key("provider-a", &format!("app-{i}")),
            &large,
        )
        .unwrap();
    }
    assert!(store::save(&backend, &fixture_key("provider-b", "extra"), &large).is_err());
    let restored = store::load(&backend, &fixture_key("provider-a", "app-0"))
        .unwrap()
        .unwrap();
    assert_eq!(restored.storage, large.storage);
    store::remove(&backend, &fixture_key("provider-a", "app-0")).unwrap();
    store::save(&backend, &fixture_key("provider-b", "extra"), &large).unwrap();
}

#[test]
fn malformed_legacy_record_is_preserved_for_recovery() {
    let backend = Memory::default();
    let key = fixture_key("provider-a", "app-a");
    backend.write(LEGACY_SERVICE, &key, b"not-json").unwrap();
    assert!(store::load(&backend, &key).is_err());
    assert_eq!(
        backend.read(LEGACY_SERVICE, &key).unwrap().unwrap(),
        b"not-json"
    );
    assert_eq!(backend.records.borrow().len(), 1);
}

#[cfg(target_os = "macos")]
mod processes {
    use super::*;
    use std::path::PathBuf;

    // Contains only synthetic fixtures; production never stores credential files.
    struct Files(PathBuf);
    impl Files {
        fn path(&self, service: &str, key: &str) -> PathBuf {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(format!("{service}:{key}").as_bytes());
            self.0.join(format!("{digest:x}"))
        }
    }
    impl Backend for Files {
        fn read(&self, service: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
            match std::fs::read(self.path(service, key)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        }
        fn write(&self, service: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
            std::fs::write(self.path(service, key), bytes).map_err(|e| e.to_string())
        }
        fn delete(&self, service: &str, key: &str) -> Result<(), String> {
            match std::fs::remove_file(self.path(service, key)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
    }
    #[test]
    fn vault_writer_process() {
        let Some(root) = std::env::var_os("ACHEFLOW_VAULT_TEST_DIRECTORY") else {
            return;
        };
        let backend = Files(root.into());
        let _lock = super::super::native::lock(&backend.0.join("lock")).unwrap();
        let application = std::env::var("ACHEFLOW_VAULT_TEST_SLOT").unwrap();
        store::save(
            &backend,
            &fixture_key("provider-a", &application),
            &state(&application),
        )
        .unwrap();
    }
    #[test]
    fn parallel_processes_preserve_every_account_in_one_record() {
        let root = std::env::temp_dir().join(format!(
            "acheflow-vault-processes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let backend = Files(root.clone());
        let mut children = Vec::new();
        for i in 0..12 {
            children.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "vault::workspace_tests::processes::vault_writer_process",
                    ])
                    .env("ACHEFLOW_VAULT_TEST_DIRECTORY", &root)
                    .env("ACHEFLOW_VAULT_TEST_SLOT", format!("app-{i}"))
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        let successful = children
            .iter_mut()
            .map(|c| c.wait().unwrap().success())
            .collect::<Vec<_>>();
        assert!(successful.into_iter().all(|ok| ok));
        for i in 0..12 {
            let restored = store::load(&backend, &fixture_key("provider-a", &format!("app-{i}")))
                .unwrap()
                .unwrap();
            assert_eq!(restored.secrets["access_token"], format!("app-{i}"));
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2); // Container + lock.
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn server_accounts_never_adopt_local_credentials_and_share_only_their_workspace_container() {
    let backend = Memory::default();
    let local = fixture_key("alpha", "app");
    let first = crate::server_account::storage_key(
        "server-fixture",
        "workspace-fixture",
        "alpha",
        "app",
        "bot-a",
    )
    .unwrap();
    let second = crate::server_account::storage_key(
        "server-fixture",
        "workspace-fixture",
        "beta",
        "app",
        "bot-b",
    )
    .unwrap();
    store::save(&backend, &local, &state("local-token")).unwrap();
    assert!(store::load(&backend, &first).unwrap().is_none());
    store::save(&backend, &first, &state("server-a")).unwrap();
    store::save(&backend, &second, &state("server-b")).unwrap();
    store::save(&backend, &first, &state("server-a-renewed")).unwrap();
    assert_token(&backend, &local, "local-token");
    assert_token(&backend, &first, "server-a-renewed");
    assert_token(&backend, &second, "server-b");
    assert_eq!(backend.records.borrow().len(), 2);
}

#[test]
fn endpoint_alias_migration_preserves_conflicts_neighbors_and_failed_writes() {
    let backend = Memory::default();
    let old = fixture_key("alpha", "app");
    let neighbor = fixture_key("beta", "app");
    let stable = key(
        "client:verified-server",
        "workspace-fixture",
        "alpha",
        "app",
    )
    .unwrap();
    store::save(&backend, &old, &state("old")).unwrap();
    store::save(&backend, &neighbor, &state("neighbor")).unwrap();
    backend.fail_write.set(true);
    assert!(store::migrate(&backend, &old, &stable).is_err());
    backend.fail_write.set(false);
    assert_token(&backend, &old, "old");
    store::migrate(&backend, &old, &stable).unwrap();
    assert!(store::load(&backend, &old).unwrap().is_none());
    assert_token(&backend, &stable, "old");
    assert_token(&backend, &neighbor, "neighbor");
    store::save(&backend, &old, &state("different")).unwrap();
    store::migrate(&backend, &old, &stable).unwrap();
    assert_token(&backend, &old, "different");
    assert_token(&backend, &stable, "old");
}

#[test]
fn migrated_slots_stop_reopening_per_extension_records_and_identical_saves_do_not_write() {
    let backend = Memory::default();
    let first = fixture_key("alpha", "app");
    let second = fixture_key("beta", "app");
    legacy(&backend, &first, "one");
    assert_token(&backend, &first, "one");
    store::save(&backend, &second, &state("two")).unwrap();
    let reads = backend.legacy_reads.get();
    let writes = backend.writes.get();
    for _ in 0..3 {
        assert_token(&backend, &first, "one");
        assert_token(&backend, &second, "two");
        store::save(&backend, &first, &state("one")).unwrap();
    }
    assert_eq!(
        backend.legacy_reads.get(),
        reads,
        "retired records must not be reopened"
    );
    assert_eq!(
        backend.writes.get(),
        writes,
        "unchanged credentials must not invalidate the OS cache"
    );
    assert_eq!(backend.records.borrow().len(), 1);
}
