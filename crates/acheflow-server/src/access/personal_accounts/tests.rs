use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
#[derive(Default)]
struct Memory {
    slots: Mutex<BTreeSet<(String, String)>>,
    writes: AtomicUsize,
    locked: AtomicBool,
    fail_remove: AtomicBool,
}
impl Store for Memory {
    fn save(
        &self,
        a: &AuthorizedAccount,
        _: &str,
        b: &StoredPersonalAccount,
    ) -> Result<(), String> {
        if self.locked.load(Ordering::SeqCst) {
            return Err("locked".into());
        }
        assert_eq!(a.account_id(), b.account_id);
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.slots
            .lock()
            .unwrap()
            .insert((b.user.clone(), b.slot.clone()));
        Ok(())
    }
    fn remove(&self, _: &str, b: &StoredPersonalAccount) -> Result<(), String> {
        if self.fail_remove.load(Ordering::SeqCst) {
            return Err("locked".into());
        }
        self.slots
            .lock()
            .unwrap()
            .remove(&(b.user.clone(), b.slot.clone()));
        Ok(())
    }
}
fn artifact(provider: &str) -> acheflow_client::catalog::Installed {
    acheflow_client::catalog::Installed {manifest:serde_json::from_value(serde_json::json!({"id":provider,"name":"Fixture","api_version":2,"module":"fixture.wasm","execution_side":"client","services":[],"capabilities":["secret_commit","storage"],"secret_fields":{"access_token":"/access_token"},"settings":[{"id":"server_authorization_fixture","label":"Fixture"}]})).unwrap(),module:include_bytes!("../../../tests/fixtures/domain.wasm").to_vec(),browser_auth:None}
}
fn fixture() -> (
    super::super::tests::Directory,
    Arc<Access>,
    Principal,
    String,
) {
    let directory = super::super::tests::Directory::new();
    let (mut host, workspace) = crate::control::tests::host();
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.clear();
    inner.provider_catalog.installed.clear();
    for provider in ["alpha", "beta"] {
        inner
            .provider_catalog
            .installed
            .insert(provider.into(), artifact(provider));
    }
    let access = Access::open(directory.path(), host).unwrap();
    let owner = Principal(
        access
            .registry
            .authenticate(&directory.owner(), now())
            .unwrap(),
    );
    (directory, access, owner, workspace.id)
}
async fn verified(provider: &str) -> AuthorizedAccount {
    let a = artifact(provider);
    acheflow_client::server_account::authorization::authorize(
        a.manifest,
        &a.module,
        BTreeMap::from([("server_authorization_fixture".into(), "true".into())]),
        BTreeMap::from([("access_token".into(), "synthetic-token".into())]),
    )
    .await
    .unwrap()
}
fn pending(actor: &Principal, workspace: &str, provider: &str) -> StoredPersonalAccount {
    StoredPersonalAccount {
        workspace: workspace.into(),
        user: actor.0.user.id.clone(),
        provider: provider.into(),
        account_id: "verified-bot".into(),
        slot: registry::random(16).unwrap(),
        version: registry::random(16).unwrap(),
        config: Default::default(),
    }
}
fn participant(access: &Access, owner: &Principal) -> Principal {
    let code = access
        .registry
        .invite(&owner.0, [P::StudioView].into(), now())
        .unwrap();
    Principal(
        access
            .registry
            .pair(&code.code, "Same name", "fixture", now())
            .unwrap()
            .session,
    )
}
#[tokio::test]
async fn own_accounts_are_shared_only_by_explicitly_linked_devices_and_never_start_collectors() {
    let (dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let user = participant(&access, &owner);
    let other = participant(&access, &owner);
    let before = access.host.runtime.saved_state();
    for provider in ["alpha", "beta"] {
        complete(
            &access,
            &user,
            pending(&user, &workspace, provider),
            None,
            verified(provider).await,
            &store,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        access
            .registry
            .personal_accounts(&user.0, &workspace, now())
            .unwrap()
            .len(),
        2
    );
    assert!(
        access
            .registry
            .personal_accounts(&other.0, &workspace, now())
            .unwrap()
            .is_empty()
    );
    assert!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace, now())
            .unwrap()
            .is_empty(),
        "HTTP administrators do not inherit personal account ownership"
    );
    let link = access
        .registry
        .link_invite(&user.0, [P::StudioView].into(), now())
        .unwrap();
    let linked = access
        .registry
        .pair(&link.code, "Own phone", "linked", now())
        .unwrap();
    assert_eq!(
        access
            .registry
            .personal_accounts(&linked.session, &workspace, now())
            .unwrap()
            .len(),
        2
    );
    assert_eq!(before, access.host.runtime.saved_state());
    assert!(!access.host.providers.active().await);
    assert!(access.host.snapshot(0, 10).client_executors.is_empty());
    assert!(
        access
            .host
            .snapshot(0, 10)
            .server_providers
            .iter()
            .all(|p| !p.server_owned && p.generation == 0)
    );
    let first = access
        .registry
        .personal_accounts(&user.0, &workspace, now())
        .unwrap()[0]
        .clone();
    assert_eq!(
        disconnect_with(
            &access,
            &other,
            &workspace,
            &first.provider,
            &first.version,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        404
    );
    disconnect_with(
        &access,
        &Principal(linked.session.clone()),
        &workspace,
        &first.provider,
        &first.version,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(
        access
            .registry
            .personal_accounts(&user.0, &workspace, now())
            .unwrap()
            .len(),
        1
    );
    let token = linked.token.unwrap();
    drop(access);
    let (host, _) = crate::control::tests::host();
    let restarted = Access::open(dir.path(), host).unwrap();
    let session = restarted.registry.authenticate(&token, now()).unwrap();
    assert_eq!(
        restarted
            .registry
            .personal_accounts(&session, &workspace, now())
            .unwrap()
            .len(),
        1
    );
    let json = serde_json::to_string(
        &restarted
            .registry
            .personal_accounts(&session, &workspace, now())
            .unwrap(),
    )
    .unwrap();
    assert!(
        !json.contains("slot") && !json.contains("config") && !json.contains("synthetic-token")
    );
}
#[tokio::test]
async fn replacement_rechecks_version_owner_workspace_and_revocation_before_vault_write() {
    let (dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let user = participant(&access, &owner);
    let first = complete(
        &access,
        &user,
        pending(&user, &workspace, "alpha"),
        None,
        verified("alpha").await,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(
        complete(
            &access,
            &user,
            pending(&user, &workspace, "alpha"),
            None,
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        409
    );
    let other = participant(&access, &owner);
    assert_eq!(
        complete(
            &access,
            &other,
            pending(&user, &workspace, "alpha"),
            Some(&first.version),
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        403
    );
    let changed = pending(&user, &workspace, "alpha");
    access
        .registry
        .revoke(&owner.0, &user.0.device.id, now())
        .unwrap();
    assert_eq!(
        complete(
            &access,
            &user,
            changed,
            Some(&first.version),
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        403
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    let mut foreign = pending(&owner, &workspace, "alpha");
    foreign.workspace = "different-workspace".into();
    assert_eq!(
        complete(
            &access,
            &owner,
            foreign,
            None,
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        409
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    // Session expiration is also rechecked after validation.
    let expired = access
        .registry
        .pair(&dir.owner(), "Expired", "expired", 1)
        .unwrap();
    let expired = Principal(expired.session);
    assert_eq!(
        complete(
            &access,
            &expired,
            pending(&expired, &workspace, "alpha"),
            None,
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        403
    );
}
#[tokio::test]
async fn locked_vault_and_failed_metadata_save_preserve_the_old_account() {
    let (dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let first = complete(
        &access,
        &owner,
        pending(&owner, &workspace, "alpha"),
        None,
        verified("alpha").await,
        &store,
    )
    .await
    .unwrap();
    let oldslots = store.slots.lock().unwrap().clone();
    store.locked.store(true, Ordering::SeqCst);
    assert_eq!(
        complete(
            &access,
            &owner,
            pending(&owner, &workspace, "alpha"),
            Some(&first.version),
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        503
    );
    store.locked.store(false, Ordering::SeqCst);
    let bytes = std::fs::read(dir.path()).unwrap();
    std::fs::remove_file(dir.path()).unwrap();
    std::fs::create_dir(dir.path()).unwrap();
    std::fs::write(dir.path().join("blocker"), b"fixture").unwrap();
    assert_eq!(
        complete(
            &access,
            &owner,
            pending(&owner, &workspace, "alpha"),
            Some(&first.version),
            verified("alpha").await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        503
    );
    assert_eq!(*store.slots.lock().unwrap(), oldslots);
    assert_eq!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace, now())
            .unwrap(),
        vec![first.clone()]
    );
    std::fs::remove_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path(), bytes).unwrap();
    let second = complete(
        &access,
        &owner,
        pending(&owner, &workspace, "alpha"),
        Some(&first.version),
        verified("alpha").await,
        &store,
    )
    .await
    .unwrap();
    assert_ne!(second.version, first.version);
    assert_eq!(store.slots.lock().unwrap().len(), 1);
    assert_eq!(
        disconnect_with(&access, &owner, &workspace, "alpha", &first.version, &store)
            .await
            .err()
            .unwrap()
            .status(),
        409
    );
    store.fail_remove.store(true, Ordering::SeqCst);
    assert!(
        !disconnect_with(
            &access,
            &owner,
            &workspace,
            "alpha",
            &second.version,
            &store
        )
        .await
        .unwrap()
        .credentials_removed
    );
    assert!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace, now())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn http_ownership_comes_from_session_and_viewers_can_manage_only_their_own_accounts() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let (_dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let user = participant(&access, &owner);
    let other = participant(&access, &owner);
    let account = complete(
        &access,
        &user,
        pending(&user, &workspace, "alpha"),
        None,
        verified("alpha").await,
        &store,
    )
    .await
    .unwrap();
    let app = access.protect(access.routes());
    let call = |method: &str, path: String, who: Principal, body: serde_json::Value, csrf: bool| {
        // Each test-only token is a normal one-use link to the selected user, never forged headers.
        let link = access
            .registry
            .link_invite(&who.0, [P::StudioView].into(), now())
            .unwrap();
        let paired = access
            .registry
            .pair(
                &link.code,
                "HTTP fixture",
                &registry::random(8).unwrap(),
                now(),
            )
            .unwrap();
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "studio.example")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", paired.token.unwrap()));
        if csrf {
            request = request.header("x-acheflow-request", "1");
        }
        app.clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
    };
    let route = format!("/api/access/personal-accounts/{workspace}");
    let response = call(
        "GET",
        route.clone(),
        user.clone(),
        serde_json::json!({}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["accounts"][0]["account_id"], "verified-bot");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("slot")
            && !String::from_utf8_lossy(&bytes).contains("config")
    );
    for outsider in [owner.clone(), other.clone()] {
        let response = call(
            "GET",
            route.clone(),
            outsider.clone(),
            serde_json::json!({}),
            true,
        )
        .await
        .unwrap();
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["accounts"], serde_json::json!([]));
        assert_eq!(
            call(
                "DELETE",
                format!("{route}/alpha"),
                outsider,
                serde_json::json!({"expected_version":account.version}),
                true
            )
            .await
            .unwrap()
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    let body = serde_json::json!({"trust_server":true,"expected_account":"wrong-platform-user","expected_version":null,"config":{"server_authorization_fixture":"true"},"credentials":{"access_token":"synthetic-sensitive-token"}});
    let response = call(
        "POST",
        format!("{route}/beta"),
        user.clone(),
        body.clone(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !String::from_utf8_lossy(&to_bytes(response.into_body(), 8192).await.unwrap())
            .contains("synthetic-sensitive-token")
    );
    let mut forged = body.clone();
    forged["user_id"] = owner.0.user.id.clone().into();
    assert_eq!(
        call("POST", format!("{route}/beta"), user.clone(), forged, true)
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call("POST", format!("{route}/beta"), user.clone(), body, false)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let request = Request::builder().uri(route).body(Body::empty()).unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn legacy_metadata_defaults_empty_and_corrupt_ownership_is_rejected_without_reset() {
    let (dir, access, owner, workspace) = fixture();
    let key = dir.owner();
    drop(access);
    let mut disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path()).unwrap()).unwrap();
    disk.as_object_mut().unwrap().remove("personal_accounts");
    std::fs::write(dir.path(), serde_json::to_vec(&disk).unwrap()).unwrap();
    let (host, _) = crate::control::tests::host();
    let access = Access::open(dir.path(), host).unwrap();
    assert_eq!(
        access.registry.authenticate(&key, now()).unwrap().user,
        owner.0.user
    );
    assert!(
        access
            .registry
            .personal_accounts(&owner.0, &workspace, now())
            .unwrap()
            .is_empty()
    );
    access
        .registry
        .set_personal(&owner.0, pending(&owner, &workspace, "alpha"), None, now())
        .unwrap();
    drop(access);
    let mut disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path()).unwrap()).unwrap();
    disk["personal_accounts"][0]["user"] = "missing-owner".into();
    let bytes = serde_json::to_vec(&disk).unwrap();
    std::fs::write(dir.path(), &bytes).unwrap();
    let (host, _) = crate::control::tests::host();
    assert!(Access::open(dir.path(), host).is_err());
    assert_eq!(std::fs::read(dir.path()).unwrap(), bytes);
}
