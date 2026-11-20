use super::*;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
struct Memory {
    slots: Mutex<BTreeSet<String>>,
    fail_write: AtomicBool,
}
impl AccountStore for Memory {
    fn save(
        &self,
        account: &authorization::AuthorizedAccount,
        _: &str,
        pending: &PendingAccount,
    ) -> Result<(), String> {
        assert_eq!(account.account_id(), pending.binding.account_id);
        if self.fail_write.load(Ordering::SeqCst) {
            return Err("locked".into());
        }
        self.slots
            .lock()
            .unwrap()
            .insert(pending.binding.application.clone());
        Ok(())
    }
    fn remove(&self, _: &str, _: &str, _: &str, binding: &ServerIntegration) -> Result<(), String> {
        self.slots.lock().unwrap().remove(&binding.application);
        Ok(())
    }
}
fn artifact() -> acheflow_client::catalog::Installed {
    acheflow_client::catalog::Installed {
        manifest: serde_json::from_value(serde_json::json!({"id":"alpha","name":"Fixture","api_version":2,"module":"fixture.wasm","execution_side":"client","services":[],"capabilities":["secret_commit","storage"],"secret_fields":{"access_token":"/access_token"},"settings":[{"id":"server_authorization_fixture","label":"Fixture"}]})).unwrap(),
        module: include_bytes!("../../../tests/fixtures/domain.wasm").to_vec(),browser_auth:None,
    }
}
fn fixture() -> (
    super::super::tests::Directory,
    Arc<Access>,
    Principal,
    Workspace,
) {
    let directory = super::super::tests::Directory::new();
    let (mut host, workspace) = crate::control::tests::host();
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.get_mut().unwrap().clear();
    inner.provider_catalog.get_mut().unwrap().installed.clear();
    inner
        .provider_catalog
        .get_mut()
        .unwrap()
        .installed
        .insert("alpha".into(), artifact());
    let access = Access::open(directory.path(), host).unwrap();
    let owner = Principal(
        access
            .registry
            .authenticate(&directory.owner(), now())
            .unwrap(),
    );
    (directory, access, owner, workspace)
}
async fn authorized() -> authorization::AuthorizedAccount {
    let a = artifact();
    authorization::authorize(
        a.manifest,
        &a.module,
        BTreeMap::from([("server_authorization_fixture".into(), "true".into())]),
        BTreeMap::from([("access_token".into(), "synthetic-token".into())]),
    )
    .await
    .unwrap()
}
fn pending(access: &Access, workspace: &Workspace, slot: &str) -> PendingAccount {
    PendingAccount {
        workspace: workspace.id.clone(),
        revision: access.host.runtime.saved_state().0,
        provider: "alpha".into(),
        binding: ServerIntegration {
            enabled: true,
            application: slot.into(),
            account_id: "verified-bot".into(),
            channel_id: "channel".into(),
            config: BTreeMap::from([("server_authorization_fixture".into(), "true".into())]),
        },
    }
}
#[tokio::test]
async fn connection_is_read_only_and_locked_vault_preserves_the_old_binding() {
    let (_dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let result = complete(
        &access,
        &owner,
        pending(&access, &workspace, "first"),
        authorized().await,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(result.0.binding.account_id, "verified-bot");
    let current = access.host.runtime.saved_state().1.unwrap();
    assert_eq!(current.client_capabilities.len(), 1);
    assert_eq!(
        current.client_capabilities[0].capabilities,
        [Capability::ChatRead, Capability::AudienceRead].into()
    );
    let old = current.server_integrations["alpha"].clone();
    store.fail_write.store(true, Ordering::SeqCst);
    assert_eq!(
        complete(
            &access,
            &owner,
            pending(&access, &workspace, "second"),
            authorized().await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        503
    );
    assert_eq!(
        access
            .host
            .runtime
            .saved_state()
            .1
            .unwrap()
            .server_integrations["alpha"],
        old
    );
    assert_eq!(
        *store.slots.lock().unwrap(),
        BTreeSet::from(["first".into()])
    );
    store.fail_write.store(false, Ordering::SeqCst);
    let _ = complete(
        &access,
        &owner,
        pending(&access, &workspace, "second"),
        authorized().await,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(
        *store.slots.lock().unwrap(),
        BTreeSet::from(["second".into()])
    );
}
#[tokio::test]
async fn stale_revision_and_revoked_actor_are_rechecked_after_validation() {
    let (dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let stale = pending(&access, &workspace, "stale");
    access
        .host
        .runtime
        .update_workspace(&workspace.id, |w| {
            w.name = "Changed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        complete(&access, &owner, stale, authorized().await, &store)
            .await
            .err()
            .unwrap()
            .status(),
        409
    );
    let paired = access
        .registry
        .pair(&dir.owner(), "Temporary", "fixture", now())
        .unwrap();
    let actor = Principal(paired.session);
    access
        .registry
        .revoke(&owner.0, &actor.0.device.id, now())
        .unwrap();
    assert_eq!(
        complete(
            &access,
            &actor,
            pending(&access, &workspace, "revoked"),
            authorized().await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        403
    );
    assert!(store.slots.lock().unwrap().is_empty());
}
#[tokio::test]
async fn failed_workspace_save_removes_only_the_new_slot() {
    let (dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let path = dir.path().with_extension("workspace.json");
    let disk =
        crate::workspace_store::Store::open(path.clone(), &access.registry.server_id()).unwrap();
    disk.save(workspace.edit_revision, Some(&workspace))
        .unwrap();
    access.host.runtime.attach_store(disk).unwrap();
    let _ = complete(
        &access,
        &owner,
        pending(&access, &workspace, "first"),
        authorized().await,
        &store,
    )
    .await
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        complete(
            &access,
            &owner,
            pending(&access, &workspace, "second"),
            authorized().await,
            &store
        )
        .await
        .err()
        .unwrap()
        .status(),
        503
    );
    assert_eq!(
        *store.slots.lock().unwrap(),
        BTreeSet::from(["first".into()])
    );
    assert_eq!(
        access
            .host
            .runtime
            .saved_state()
            .1
            .unwrap()
            .server_integrations["alpha"]
            .application,
        "first"
    );
}

#[tokio::test]
async fn disconnect_removes_only_the_server_slot_and_reserves_collection() {
    let (_dir, access, owner, workspace) = fixture();
    let store = Memory::default();
    let _ = complete(
        &access,
        &owner,
        pending(&access, &workspace, "server-slot"),
        authorized().await,
        &store,
    )
    .await
    .unwrap();
    store
        .slots
        .lock()
        .unwrap()
        .insert("local-account-unchanged".into());
    disconnect_with(
        &access,
        &owner,
        "alpha",
        DisconnectServerAccount {
            workspace_id: workspace.id.clone(),
            expected_revision: access.host.runtime.saved_state().0,
        },
        &store,
    )
    .await
    .unwrap();
    assert_eq!(
        *store.slots.lock().unwrap(),
        BTreeSet::from(["local-account-unchanged".into()])
    );
    let binding = access
        .host
        .runtime
        .saved_state()
        .1
        .unwrap()
        .server_integrations
        .remove("alpha")
        .unwrap();
    assert!(
        binding.enabled,
        "The binding remains server-owned to suppress client collection"
    );
}

#[tokio::test]
async fn http_requires_management_trust_and_matching_verified_account_without_echoing_secrets() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let (dir, access, owner, workspace) = fixture();
    let app = access.protect(access.routes());
    let token = dir.owner();
    let call = |body: serde_json::Value, token: String, csrf: bool| {
        let app = app.clone();
        async move {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/access/server-accounts/alpha")
                .header("host", "studio.example")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"));
            if csrf {
                request = request.header("x-acheflow-request", "1");
            }
            app.oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap()
        }
    };
    let mut body = serde_json::json!({"workspace_id":workspace.id,"expected_revision":access.host.runtime.saved_state().0,"trust_server":false,"expected_account":"wrong-account","channel_id":"channel","config":{"server_authorization_fixture":"true"},"credentials":{"access_token":"synthetic-sensitive-token"}});
    assert_eq!(
        call(body.clone(), token.clone(), true).await.status(),
        StatusCode::BAD_REQUEST
    );
    body["trust_server"] = true.into();
    assert_eq!(
        call(body.clone(), token.clone(), false).await.status(),
        StatusCode::FORBIDDEN
    );
    let response = call(body.clone(), token.clone(), true).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-sensitive-token"));
    let invitation = access
        .registry
        .invite(&owner.0, [P::StudioView].into(), now())
        .unwrap();
    let viewer = access
        .registry
        .pair(&invitation.code, "Viewer", "fixture", now())
        .unwrap();
    assert_eq!(
        call(body, viewer.token.unwrap(), true).await.status(),
        StatusCode::FORBIDDEN
    );
    assert!(
        access
            .host
            .runtime
            .saved_state()
            .1
            .unwrap()
            .server_integrations
            .is_empty()
    );
}

#[tokio::test]
async fn config_only_role_reads_and_saves_public_settings_without_scene_media_or_execution_grants()
{
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let (_dir, access, owner, workspace) = fixture();
    let mut proposed = pending(&access, &workspace, "settings-role");
    proposed.binding.enabled = false;
    let _ = complete(
        &access,
        &owner,
        proposed,
        authorized().await,
        &Memory::default(),
    )
    .await
    .unwrap();
    let invite = access
        .registry
        .invite(&owner.0, [P::ExtensionsConfigure].into(), now())
        .unwrap();
    let configured = access
        .registry
        .pair(&invite.code, "Configurator", "fixture", now())
        .unwrap();
    let token = configured.token.unwrap();
    let app = access.protect(access.routes());
    let call = |method: &'static str, path: &'static str, body: serde_json::Value| {
        let app = app.clone();
        let token = token.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "studio.example")
                    .header("authorization", format!("Bearer {token}"))
                    .header("x-acheflow-request", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    let response = call(
        "GET",
        "/api/access/server-accounts/alpha",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let settings: ServerIntegrationSettings = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        settings.workspace_id.as_deref(),
        Some(workspace.id.as_str())
    );
    assert_eq!(settings.binding.unwrap().account_id, "verified-bot");
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-token"));
    let update = serde_json::json!({"workspace_id":workspace.id,"expected_revision":settings.revision,"enabled":false,"config":{"server_authorization_fixture":"false"}});
    assert_eq!(
        call("PUT", "/api/access/server-accounts/alpha", update.clone())
            .await
            .status(),
        StatusCode::OK
    );
    let mut enable = update;
    enable["expected_revision"] = serde_json::json!(access.host.runtime.saved_state().0);
    enable["enabled"] = serde_json::json!(true);
    assert_eq!(
        call("PUT", "/api/access/server-accounts/alpha", enable)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            "GET",
            "/api/extensions/server-providers/alpha/module",
            serde_json::Value::Null
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            "POST",
            "/api/access/server-accounts/alpha",
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}
