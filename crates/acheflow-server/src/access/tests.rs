use super::registry::Registry;
use acheflow_media_api::access::{Permission as P, Session};
use std::{collections::BTreeSet, path::PathBuf};

const NOW: u64 = 1_000_000;
pub(super) struct Directory(PathBuf);
impl Directory {
    pub(super) fn new() -> Self {
        let mut bytes = [0; 16];
        openssl::rand::rand_bytes(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("acheflow-access-{name}"));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn path(&self) -> PathBuf {
        self.0.join("server.access.json")
    }
    pub(super) fn owner(&self) -> String {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(self.path()).unwrap()).unwrap()["owner_key"].as_str().unwrap().into()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn setup() -> (Directory, Registry, Session) {
    let dir = Directory::new();
    let registry = Registry::open(dir.path()).unwrap();
    let owner = registry.authenticate(&dir.owner(), NOW).unwrap();
    (dir, registry, owner)
}
fn permissions(values: &[P]) -> BTreeSet<P> {
    values.iter().copied().collect()
}

#[test]
fn invitations_are_one_use_scoped_and_device_sessions_survive_restart() {
    let (dir, registry, owner) = setup();
    let invitation = registry
        .invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    let paired = registry
        .pair(&invitation.code, "Phone", "peer", NOW)
        .unwrap();
    let token = paired.token.unwrap();
    assert_eq!(
        paired.session.device.permissions,
        permissions(&[P::StudioView])
    );
    assert!(
        registry
            .pair(&invitation.code, "Replay", "peer", NOW)
            .is_err()
    );
    let identity = paired.session.server_id;
    drop(registry);
    let restarted = Registry::open(dir.path()).unwrap();
    let session = restarted.authenticate(&token, NOW + 1).unwrap();
    assert_eq!(session.server_id, identity);
    assert_eq!(session.device.name, "Phone");
    let saved = std::fs::read_to_string(dir.path()).unwrap();
    assert!(!saved.contains(&token));
    assert!(!saved.contains(&invitation.code));
}

#[test]
fn viewers_cannot_issue_invites_or_revoke_other_devices_and_revocation_is_durable() {
    let (dir, registry, owner) = setup();
    let invitation = registry
        .invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    let paired = registry
        .pair(&invitation.code, "Reader", "peer", NOW)
        .unwrap();
    assert!(
        registry
            .invite(&paired.session, permissions(&[P::AccessManage]), NOW)
            .is_err()
    );
    assert!(
        registry
            .revoke(&paired.session, &owner.device.id, NOW)
            .is_err()
    );
    registry
        .revoke(&owner, &paired.session.device.id, NOW)
        .unwrap();
    assert!(
        registry
            .authenticate(paired.token.as_ref().unwrap(), NOW)
            .is_none()
    );
    drop(registry);
    let restarted = Registry::open(dir.path()).unwrap();
    assert!(
        restarted
            .authenticate(paired.token.as_ref().unwrap(), NOW)
            .is_none()
    );
}

#[test]
fn delegated_admin_cannot_grant_rights_they_do_not_have_and_revocation_cancels_their_invites() {
    let (_dir, registry, owner) = setup();
    let grants = permissions(&[P::AccessManage, P::StudioView]);
    let code = registry.invite(&owner, grants.clone(), NOW).unwrap().code;
    let delegated = registry.pair(&code, "Limited admin", "peer", NOW).unwrap();
    assert!(
        registry
            .invite(&delegated.session, permissions(&[P::ExtensionsRun]), NOW)
            .is_err()
    );
    let next = registry
        .invite(&delegated.session, permissions(&[P::StudioView]), NOW)
        .unwrap();
    registry
        .revoke(&owner, &delegated.session.device.id, NOW)
        .unwrap();
    assert!(registry.pair(&next.code, "Reader", "peer", NOW).is_err());
}

#[test]
fn expired_invites_and_sessions_are_rejected_and_pairing_failures_are_bounded() {
    let (_dir, registry, owner) = setup();
    let invite = registry
        .invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    assert!(
        registry
            .pair(&invite.code, "Expired", "peer", invite.expires_at_ms)
            .is_err()
    );
    let invite = registry
        .invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    let paired = registry
        .pair(&invite.code, "Reader", "other-peer", NOW)
        .unwrap();
    assert!(
        registry
            .authenticate(
                paired.token.as_ref().unwrap(),
                paired.session.device.expires_at_ms
            )
            .is_none()
    );
    for _ in 0..10 {
        assert!(
            registry
                .pair("invalid", "Unknown", "bad-peer", NOW)
                .is_err()
        );
    }
    assert_eq!(
        registry
            .pair(&invite.code, "Limited", "bad-peer", NOW)
            .err()
            .unwrap()
            .status(),
        429
    );
}

#[test]
fn another_process_cannot_open_the_same_access_store_and_corruption_is_not_reset() {
    let (dir, registry, _owner) = setup();
    assert!(Registry::open(dir.path()).is_err());
    drop(registry);
    std::fs::write(dir.path(), b"invalid").unwrap();
    assert!(Registry::open(dir.path()).is_err());
    assert_eq!(std::fs::read(dir.path()).unwrap(), b"invalid");
}

#[test]
fn failed_save_neither_consumes_an_invitation_nor_issues_a_session() {
    let (dir, registry, owner) = setup();
    let invitation = registry
        .invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    // A nonempty directory makes atomic replacement fail without touching saved data.
    let link = registry
        .link_invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    let original = std::fs::read(dir.path()).unwrap();
    std::fs::remove_file(dir.path()).unwrap();
    std::fs::create_dir(dir.path()).unwrap();
    std::fs::write(dir.path().join("blocker"), b"fixture").unwrap();
    assert!(
        registry
            .pair(&invitation.code, "Reader", "peer", NOW)
            .is_err()
    );
    assert!(registry.pair(&link.code, "Linked", "peer", NOW).is_err());
    assert!(registry.profile(&owner, "Unsaved", NOW).is_err());
    assert_eq!(
        registry
            .authenticate(
                serde_json::from_slice::<serde_json::Value>(&original).unwrap()["owner_key"]
                    .as_str()
                    .unwrap(),
                NOW
            )
            .unwrap()
            .user
            .name,
        owner.user.name
    );
    std::fs::remove_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path(), original).unwrap();
    assert!(registry.pair(&link.code, "Linked", "peer", NOW).is_ok());
    assert!(
        registry
            .pair(&invitation.code, "Reader", "peer", NOW)
            .is_ok()
    );
}

#[test]
fn linked_devices_share_identity_but_never_union_permissions() {
    let (_dir, registry, owner) = setup();
    let code = registry
        .link_invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    let phone = registry.pair(&code.code, "Phone", "phone", NOW).unwrap();
    assert_eq!(phone.session.user, owner.user);
    assert_eq!(phone.session.device.user_id, owner.user.id);
    assert_eq!(
        phone.session.device.permissions,
        permissions(&[P::StudioView])
    );
    assert!(registry.pair(&code.code, "Replay", "phone", NOW).is_err());
    assert!(
        registry
            .link_invite(&phone.session, permissions(&[P::AccessManage]), NOW)
            .is_err()
    );
    assert!(
        registry
            .invite(&phone.session, permissions(&[P::StudioView]), NOW)
            .is_err()
    );
    let code = registry
        .link_invite(&phone.session, permissions(&[P::StudioView]), NOW)
        .unwrap();
    let tablet = registry.pair(&code.code, "Tablet", "tablet", NOW).unwrap();
    assert_eq!(tablet.session.user.id, owner.user.id);
    let other = registry
        .invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    let other = registry.pair(&other.code, "Phone", "other", NOW).unwrap();
    assert_ne!(other.session.user.id, phone.session.user.id);
    assert_eq!(registry.my_devices(&phone.session, NOW).unwrap().len(), 2);
    assert_eq!(registry.my_devices(&other.session, NOW).unwrap().len(), 1);
    assert!(
        registry
            .revoke(&other.session, &phone.session.device.id, NOW)
            .is_err()
    );
    registry
        .revoke(&phone.session, &tablet.session.device.id, NOW)
        .unwrap();
    assert!(
        registry
            .authenticate(tablet.token.as_ref().unwrap(), NOW)
            .is_none()
    );
}

#[test]
fn linked_profile_is_durable_and_forged_identity_is_rejected() {
    let (dir, registry, owner) = setup();
    let code = registry
        .link_invite(&owner, permissions(&[P::ChatRead]), NOW)
        .unwrap();
    let phone = registry.pair(&code.code, "Phone", "phone", NOW).unwrap();
    let renamed = registry.profile(&phone.session, " Alice ", NOW).unwrap();
    assert_eq!(renamed.user.name, "Alice");
    assert_eq!(renamed.device.name, "Phone");
    assert_eq!(
        registry.authenticate(&dir.owner(), NOW).unwrap().user,
        renamed.user
    );
    for name in ["", "\nAlice", &"x".repeat(81)] {
        assert!(registry.profile(&phone.session, name, NOW).is_err());
    }
    let mut forged = phone.session.clone();
    forged.user.id = "0".repeat(32);
    assert!(registry.profile(&forged, "Forged", NOW).is_err());
    assert!(
        registry
            .link_invite(&forged, permissions(&[P::ChatRead]), NOW)
            .is_err()
    );
    assert!(registry.my_devices(&forged, NOW).is_err());
    assert!(
        registry
            .revoke(&forged, &phone.session.device.id, NOW)
            .is_err()
    );
    drop(registry);
    let registry = Registry::open(dir.path()).unwrap();
    assert_eq!(
        registry
            .authenticate(phone.token.as_ref().unwrap(), NOW)
            .unwrap()
            .user,
        renamed.user
    );
}

#[test]
fn revoked_or_expired_issuer_cannot_link_another_device() {
    let (_dir, registry, owner) = setup();
    let phone = registry
        .pair(
            &registry
                .link_invite(&owner, permissions(&[P::StudioView]), NOW)
                .unwrap()
                .code,
            "Phone",
            "phone",
            NOW,
        )
        .unwrap();
    let code = registry
        .link_invite(&phone.session, permissions(&[P::StudioView]), NOW)
        .unwrap();
    registry
        .revoke(&owner, &phone.session.device.id, NOW)
        .unwrap();
    assert!(registry.pair(&code.code, "Late", "late", NOW).is_err());
    let code = registry
        .link_invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    assert!(
        registry
            .pair(&code.code, "Late", "late", NOW + 600_000)
            .is_err()
    );
}

#[test]
fn legacy_migration_preserves_tokens_without_guessing_user_ownership() {
    let (dir, registry, owner) = setup();
    let a = registry.pair(&dir.owner(), "Same name", "a", NOW).unwrap();
    let b = registry.pair(&dir.owner(), "Same name", "b", NOW).unwrap();
    let invitation = registry
        .invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    drop(registry);
    let mut disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path()).unwrap()).unwrap();
    disk["version"] = 1.into();
    disk.as_object_mut().unwrap().remove("users");
    disk.as_object_mut().unwrap().remove("owner_user");
    for device in disk["devices"].as_array_mut().unwrap() {
        device["device"].as_object_mut().unwrap().remove("user_id");
    }
    for invite in disk["invitations"].as_array_mut().unwrap() {
        invite.as_object_mut().unwrap().remove("target_user");
    }
    std::fs::write(dir.path(), serde_json::to_vec(&disk).unwrap()).unwrap();
    let registry = Registry::open(dir.path()).unwrap();
    let root = registry.authenticate(&dir.owner(), NOW).unwrap();
    let a_session = registry
        .authenticate(a.token.as_ref().unwrap(), NOW)
        .unwrap();
    let b_session = registry
        .authenticate(b.token.as_ref().unwrap(), NOW)
        .unwrap();
    assert_ne!(a_session.user.id, b_session.user.id);
    assert_ne!(a_session.user.id, root.user.id);
    assert_eq!(a_session.device.id, a.session.device.id);
    assert_eq!(a_session.device.permissions, a.session.device.permissions);
    let c = registry.pair(&invitation.code, "Guest", "c", NOW).unwrap();
    assert_ne!(c.session.user.id, root.user.id);
    drop(registry);
    let registry = Registry::open(dir.path()).unwrap();
    assert_eq!(
        registry
            .authenticate(a.token.as_ref().unwrap(), NOW)
            .unwrap()
            .user,
        a_session.user
    );
    drop(registry);
    let mut disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path()).unwrap()).unwrap();
    disk["devices"][0]["device"]["user_id"] = "missing".into();
    let bytes = serde_json::to_vec(&disk).unwrap();
    std::fs::write(dir.path(), &bytes).unwrap();
    assert!(Registry::open(dir.path()).is_err());
    assert_eq!(std::fs::read(dir.path()).unwrap(), bytes);
}

#[test]
fn own_session_revocation_does_not_require_or_inherit_its_grants() {
    let (dir, registry, owner) = setup();
    let admin = registry
        .pair(&dir.owner(), "Own desktop", "desktop", NOW)
        .unwrap();
    let link = registry
        .link_invite(&owner, permissions(&[P::StudioView]), NOW)
        .unwrap();
    let phone = registry
        .pair(&link.code, "Own phone", "phone", NOW)
        .unwrap();
    registry
        .revoke(&phone.session, &admin.session.device.id, NOW)
        .unwrap();
    assert!(
        registry
            .authenticate(admin.token.as_ref().unwrap(), NOW)
            .is_none()
    );
    assert_eq!(
        registry
            .authenticate(phone.token.as_ref().unwrap(), NOW)
            .unwrap()
            .device
            .permissions,
        permissions(&[P::StudioView])
    );
    assert!(registry.revoke(&phone.session, "owner", NOW).is_err());
}

#[test]
fn upload_permission_migration_preserves_admin_roles_without_expanding_install_only_devices() {
    let (directory, registry, owner) = setup();
    let admin = registry
        .invite(
            &owner,
            permissions(&[P::AccessManage, P::ExtensionsInstall]),
            NOW,
        )
        .unwrap();
    let admin = registry
        .pair(&admin.code, "Administrator", "fixture", NOW)
        .unwrap();
    let installer = registry
        .invite(&owner, permissions(&[P::ExtensionsInstall]), NOW)
        .unwrap();
    let installer = registry
        .pair(&installer.code, "Installer", "fixture", NOW)
        .unwrap();
    drop(registry);
    let path = directory.path();
    let mut disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    disk["version"] = 2.into();
    std::fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
    let restored = Registry::open(path).unwrap();
    assert!(
        restored
            .authenticate(admin.token.as_deref().unwrap(), NOW + 1)
            .unwrap()
            .device
            .permissions
            .contains(&P::ExtensionsUpload)
    );
    assert!(
        !restored
            .authenticate(installer.token.as_deref().unwrap(), NOW + 1)
            .unwrap()
            .device
            .permissions
            .contains(&P::ExtensionsUpload)
    );
}
