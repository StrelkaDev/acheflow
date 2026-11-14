use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Fake {
    actor: Principal,
    prepares: AtomicUsize,
    writes: AtomicUsize,
    revoke: bool,
    disconnect: bool,
    fail: bool,
    outcome: ExecutionOutcome,
}
impl Engine for Fake {
    type Ready = ();
    async fn prepare(
        &self,
        access: &Access,
        b: &StoredPersonalAccount,
        _: &str,
    ) -> Result<(), String> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        if self.revoke {
            access
                .registry
                .revoke(&self.actor.0, &self.actor.0.device.id, now())
                .unwrap();
        }
        if self.disconnect {
            access
                .registry
                .remove_personal(&self.actor.0, &b.workspace, &b.provider, &b.version, now())
                .unwrap();
        }
        if self.fail {
            Err("vault unavailable".into())
        } else {
            Ok(())
        }
    }
    async fn execute(&self, _: (), _: &str, _: &str) -> ExecutionOutcome {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.outcome
    }
}
fn fixture(
    send: bool,
) -> (
    super::super::tests::Directory,
    Arc<Access>,
    Principal,
    String,
    String,
) {
    let dir = super::super::tests::Directory::new();
    let (host, w) = crate::control::tests::host();
    host.runtime
        .workspace
        .write()
        .unwrap()
        .as_mut()
        .unwrap()
        .server_integrations
        .insert(
            "alpha".into(),
            acheflow_media_api::server_provider::ServerIntegration {
                enabled: true,
                application: "shared-slot".into(),
                account_id: "shared-bot".into(),
                channel_id: "channel".into(),
                config: Default::default(),
            },
        );
    host.runtime.activated.store(true, Ordering::Release);
    let access = Access::open(dir.path(), host).unwrap();
    let owner = access.registry.authenticate(&dir.owner(), now()).unwrap();
    let mut grants = std::collections::BTreeSet::from([P::StudioView]);
    if send {
        grants.insert(P::ChatSend);
    }
    let invitation = access.registry.invite(&owner, grants, now()).unwrap();
    let paired = access
        .registry
        .pair(&invitation.code, "User", "fixture", now())
        .unwrap();
    let actor = Principal(paired.session);
    let version = registry::random(16).unwrap();
    access
        .registry
        .set_personal(
            &actor.0,
            StoredPersonalAccount {
                workspace: w.id.clone(),
                user: actor.0.user.id.clone(),
                provider: "alpha".into(),
                account_id: "personal-user".into(),
                slot: registry::random(16).unwrap(),
                version: version.clone(),
                config: Default::default(),
            },
            None,
            now(),
        )
        .unwrap();
    (dir, access, actor, w.id, version)
}
fn engine(actor: &Principal) -> Fake {
    Fake {
        actor: actor.clone(),
        prepares: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
        revoke: false,
        disconnect: false,
        fail: false,
        outcome: ExecutionOutcome::Sent,
    }
}
fn request(access: &Access, version: &str) -> SendPersonalChat {
    SendPersonalChat {
        request_id: registry::random(16).unwrap(),
        epoch: access.personal_sends.lock().unwrap().epoch.clone(),
        issued_at_ms: now(),
        expected_version: version.into(),
        text: "Personal message".into(),
    }
}
#[tokio::test]
async fn personal_send_has_no_collector_and_replays_never_repeat_writes() {
    let (_dir, access, actor, workspace, version) = fixture(true);
    let engine = engine(&actor);
    let r = request(&access, &version);
    for _ in 0..2 {
        assert_eq!(
            send_with(
                &access,
                &actor,
                (&workspace, "alpha", "channel"),
                r.clone(),
                &engine
            )
            .await
            .unwrap()
            .outcome,
            ExecutionOutcome::Sent
        );
    }
    assert_eq!(engine.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(engine.writes.load(Ordering::SeqCst), 1);
    assert!(!access.host.providers.active().await);
    assert!(access.host.snapshot(0, 0).client_executors.is_empty());
    let mut changed = r;
    changed.text = "Different message".into();
    assert_eq!(
        send_with(
            &access,
            &actor,
            (&workspace, "alpha", "channel"),
            changed,
            &engine
        )
        .await
        .err()
        .unwrap()
        .status(),
        409
    );
}
#[tokio::test]
async fn permission_account_version_and_channel_are_checked_without_shared_account_fallback() {
    for mode in ["grant", "version", "channel", "account", "vault"] {
        let (_dir, access, actor, workspace, version) = fixture(mode != "grant");
        let mut engine = engine(&actor);
        engine.fail = mode == "vault";
        let mut r = request(&access, &version);
        if mode == "version" {
            r.expected_version = "stale".into();
        }
        if mode == "account" {
            access
                .registry
                .remove_personal(&actor.0, &workspace, "alpha", &version, now())
                .unwrap();
        }
        let result = send_with(
            &access,
            &actor,
            (
                &workspace,
                "alpha",
                if mode == "channel" {
                    "other"
                } else {
                    "channel"
                },
            ),
            r,
            &engine,
        )
        .await;
        if mode == "vault" {
            assert_eq!(result.unwrap().outcome, ExecutionOutcome::Rejected);
        } else {
            assert!(result.is_err());
            assert_eq!(engine.prepares.load(Ordering::SeqCst), 0);
        }
        assert_eq!(engine.writes.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn revocation_during_preparation_prevents_the_external_write() {
    for disconnect in [true, false] {
        let (_dir, access, actor, workspace, version) = fixture(true);
        let mut engine = engine(&actor);
        engine.disconnect = disconnect;
        engine.revoke = !disconnect;
        assert_eq!(
            send_with(
                &access,
                &actor,
                (&workspace, "alpha", "channel"),
                request(&access, &version),
                &engine
            )
            .await
            .unwrap()
            .outcome,
            ExecutionOutcome::Rejected
        );
        assert_eq!(engine.writes.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn uncertain_outcome_restart_epoch_and_expired_request_never_resend() {
    let (_dir, access, actor, workspace, version) = fixture(true);
    let mut engine = engine(&actor);
    engine.outcome = ExecutionOutcome::Unknown;
    let r = request(&access, &version);
    for _ in 0..2 {
        assert_eq!(
            send_with(
                &access,
                &actor,
                (&workspace, "alpha", "channel"),
                r.clone(),
                &engine
            )
            .await
            .unwrap()
            .outcome,
            ExecutionOutcome::Unknown
        );
    }
    assert_eq!(engine.writes.load(Ordering::SeqCst), 1);
    for expired in [true, false] {
        let mut r = request(&access, &version);
        if expired {
            r.issued_at_ms = 0;
        } else {
            r.epoch = "old-process".into();
        }
        assert_eq!(
            send_with(
                &access,
                &actor,
                (&workspace, "alpha", "channel"),
                r,
                &engine
            )
            .await
            .err()
            .unwrap()
            .status(),
            409
        );
    }
    assert_eq!(engine.writes.load(Ordering::SeqCst), 1);
}
