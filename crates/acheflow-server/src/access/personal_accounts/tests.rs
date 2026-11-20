use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Memory {
    removed: AtomicUsize,
    locked: bool,
}
impl Store for Memory {
    fn remove(&self, _: &str, _: &StoredPersonalAccount) -> Result<(), String> {
        self.removed.fetch_add(1, Ordering::SeqCst);
        if self.locked {
            Err("locked".into())
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn legacy_account_cleanup_remains_user_owned_and_retryable_when_vault_is_locked() {
    let directory = super::super::tests::Directory::new();
    let (host, workspace) = crate::control::tests::host();
    let access = Access::open(directory.path(), host).unwrap();
    let owner = Principal(
        access
            .registry
            .authenticate(&directory.owner(), now())
            .unwrap(),
    );
    let binding = StoredPersonalAccount {
        workspace: workspace.id.clone(),
        user: owner.0.user.id.clone(),
        provider: "alpha".into(),
        account_id: "old-account".into(),
        slot: "a".repeat(32),
        version: "b".repeat(32),
        config: Default::default(),
    };
    access
        .registry
        .set_personal(&owner.0, binding.clone(), None, now())
        .unwrap();
    let store = Memory {
        removed: AtomicUsize::new(0),
        locked: true,
    };
    assert!(
        disconnect_with(
            &access,
            &owner,
            &workspace.id,
            "alpha",
            &binding.version,
            &store
        )
        .await
        .is_err()
    );
    assert_eq!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace.id, now())
            .unwrap()
            .len(),
        1
    );
    let store = Memory {
        removed: AtomicUsize::new(0),
        locked: false,
    };
    let outcome = disconnect_with(
        &access,
        &owner,
        &workspace.id,
        "alpha",
        &binding.version,
        &store,
    )
    .await
    .unwrap();
    assert!(outcome.credentials_removed);
    assert!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace.id, now())
            .unwrap()
            .is_empty()
    );
    assert!(
        disconnect_with(
            &access,
            &owner,
            &workspace.id,
            "alpha",
            &binding.version,
            &store
        )
        .await
        .is_err()
    );
    assert_eq!(store.removed.load(Ordering::SeqCst), 1);
    assert_eq!(retired().await.0, 410);
}
