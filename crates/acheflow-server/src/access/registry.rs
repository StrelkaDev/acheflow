mod personal;
mod users;
use acheflow_media_api::access::{Device, Invitation, PairResponse, Permission, Session, User};
pub(super) use personal::StoredPersonalAccount;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};
use subtle::ConstantTimeEq;

const SESSION_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const INVITE_MS: u64 = 10 * 60 * 1000;
const ATTEMPT_MS: u64 = 15 * 60 * 1000;

pub(super) struct Error(pub u16, pub &'static str);
impl Error {
    pub fn status(&self) -> u16 {
        self.0
    }
}
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AccessError")
            .field(&self.0)
            .field(&self.1)
            .finish()
    }
}
type Result<T> = std::result::Result<T, Error>;
fn unavailable() -> Error {
    Error(503, "Device access storage is unavailable.")
}
fn forbidden() -> Error {
    Error(403, "This device does not have permission.")
}
pub(super) fn random(size: usize) -> Result<String> {
    let mut bytes = vec![0; size];
    openssl::rand::rand_bytes(&mut bytes).map_err(|_| unavailable())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn hash(value: &str) -> String {
    openssl::sha::sha256(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn hexadecimal(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

// No Debug: owner authority and hashed credentials never enter diagnostics.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDevice {
    device: Device,
    token_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInvite {
    token_hash: String,
    permissions: BTreeSet<Permission>,
    issuer: String,
    expires_at_ms: u64,
    #[serde(default)]
    target_user: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disk {
    version: u32,
    #[serde(default)]
    users: Vec<User>,
    #[serde(default)]
    owner_user: String,
    server_id: String,
    owner_key: String,
    devices: Vec<StoredDevice>,
    invitations: Vec<StoredInvite>,
    #[serde(default)]
    personal_accounts: Vec<StoredPersonalAccount>,
}
impl Disk {
    fn session(&self, id: &str, now: u64) -> Option<Session> {
        let device = if id == "owner" {
            Device {
                user_id: self.owner_user.clone(),
                id: "owner".into(),
                name: "Local server owner".into(),
                permissions: Permission::ALL.into_iter().collect(),
                expires_at_ms: u64::MAX,
            }
        } else {
            self.devices
                .iter()
                .find(|d| d.device.id == id && d.device.expires_at_ms > now)?
                .device
                .clone()
        };
        Some(Session {
            user: self.users.iter().find(|u| u.id == device.user_id)?.clone(),
            server_id: self.server_id.clone(),
            device,
        })
    }
    fn current(&self, actor: &Session, now: u64) -> Result<Session> {
        let current = self.session(&actor.device.id, now).ok_or_else(forbidden)?;
        if current.server_id != actor.server_id || current.user.id != actor.user.id {
            return Err(forbidden());
        }
        Ok(current)
    }
    fn administrator(&self, actor: &Session, now: u64) -> Result<Session> {
        let session = self.current(actor, now)?;
        if !session
            .device
            .permissions
            .contains(&Permission::AccessManage)
        {
            return Err(forbidden());
        }
        Ok(session)
    }
}
struct Attempts {
    started: u64,
    count: u8,
}
struct Inner {
    disk: Disk,
    attempts: HashMap<String, Attempts>,
}
pub(super) struct Registry {
    path: PathBuf,
    inner: Mutex<Inner>,
    _lock: File,
}
impl Registry {
    pub fn current(&self, actor: &Session, now: u64) -> Result<Session> {
        self.inner
            .lock()
            .map_err(|_| unavailable())?
            .disk
            .current(actor, now)
    }
    pub fn server_id(&self) -> String {
        self.inner.lock().unwrap().disk.server_id.clone()
    }
    pub fn open(path: PathBuf) -> Result<Self> {
        let parent = path.parent().ok_or_else(unavailable)?;
        std::fs::create_dir_all(parent).map_err(|_| unavailable())?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(path.with_extension("lock"))
            .map_err(|_| unavailable())?;
        lock.try_lock()
            .map_err(|_| Error(503, "Another server owns this access store."))?;
        let disk = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > 1024 * 1024 {
                    return Err(unavailable());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o077 != 0 {
                        return Err(Error(
                            503,
                            "Access store permissions must be private (0600).",
                        ));
                    }
                }
                let bytes = std::fs::read(&path).map_err(|_| unavailable())?;
                let mut disk: Disk = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
                if !matches!(disk.version, 1 | 2)
                    || !hexadecimal(&disk.server_id, 32)
                    || !hexadecimal(&disk.owner_key, 64)
                    || disk.devices.len() > 128
                    || disk.invitations.len() > 64
                {
                    return Err(unavailable());
                }
                let mut ids = BTreeSet::new();
                for device in &disk.devices {
                    if !hexadecimal(&device.device.id, 16)
                        || !hexadecimal(&device.token_hash, 64)
                        || !ids.insert(&device.device.id)
                    {
                        return Err(unavailable());
                    }
                }
                disk.validate_users()?;
                disk.validate_personal()?;
                if disk.version == 1 {
                    disk.migrate_users()?;
                    persist(&path, &disk)?;
                }
                disk
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let user = User {
                    id: random(16)?,
                    name: "Server owner".into(),
                };
                let disk = Disk {
                    version: 2,
                    owner_user: user.id.clone(),
                    users: vec![user],
                    server_id: random(16)?,
                    owner_key: random(32)?,
                    devices: vec![],
                    invitations: vec![],
                    personal_accounts: vec![],
                };
                persist(&path, &disk)?;
                disk
            }
            Err(_) => return Err(unavailable()),
        };
        Ok(Self {
            path,
            inner: Mutex::new(Inner {
                disk,
                attempts: HashMap::new(),
            }),
            _lock: lock,
        })
    }
    pub fn authenticate(&self, token: &str, now: u64) -> Option<Session> {
        if !hexadecimal(token, 64) {
            return None;
        }
        let inner = self.inner.lock().ok()?;
        if bool::from(token.as_bytes().ct_eq(inner.disk.owner_key.as_bytes())) {
            return inner.disk.session("owner", now);
        }
        let digest = hash(token);
        let device = inner
            .disk
            .devices
            .iter()
            .find(|d| bool::from(d.token_hash.as_bytes().ct_eq(digest.as_bytes())))?;
        inner.disk.session(&device.device.id, now)
    }
    pub fn invite(
        &self,
        actor: &Session,
        permissions: BTreeSet<Permission>,
        now: u64,
    ) -> Result<Invitation> {
        self.create_invite(actor, permissions, false, now)
    }
    pub fn link_invite(
        &self,
        actor: &Session,
        permissions: BTreeSet<Permission>,
        now: u64,
    ) -> Result<Invitation> {
        self.create_invite(actor, permissions, true, now)
    }
    fn create_invite(
        &self,
        actor: &Session,
        permissions: BTreeSet<Permission>,
        link: bool,
        now: u64,
    ) -> Result<Invitation> {
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = if link {
            inner.disk.current(actor, now)?
        } else {
            inner.disk.administrator(actor, now)?
        };
        if permissions.is_empty() || !permissions.is_subset(&actor.device.permissions) {
            return Err(forbidden());
        }
        let mut next = inner.disk.clone();
        next.invitations.retain(|i| i.expires_at_ms > now);
        if next.invitations.len() >= 64 {
            return Err(Error(409, "Invitation limit reached."));
        }
        let code = random(16)?;
        let expires_at_ms = now.saturating_add(INVITE_MS);
        next.invitations.push(StoredInvite {
            token_hash: hash(&code),
            permissions,
            target_user: link.then_some(actor.user.id),
            issuer: actor.device.id,
            expires_at_ms,
        });
        persist(&self.path, &next)?;
        inner.disk = next;
        Ok(Invitation {
            code,
            expires_at_ms,
        })
    }
    pub fn pair(&self, code: &str, name: &str, peer: &str, now: u64) -> Result<PairResponse> {
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        inner
            .attempts
            .retain(|_, a| now < a.started.saturating_add(ATTEMPT_MS));
        if inner.attempts.len() >= 256 && !inner.attempts.contains_key(peer) {
            return Err(Error(429, "Too many pairing attempts. Try later."));
        }
        let attempt = inner.attempts.entry(peer.into()).or_insert(Attempts {
            started: now,
            count: 0,
        });
        if attempt.count >= 10 {
            return Err(Error(429, "Too many pairing attempts. Try later."));
        }
        attempt.count += 1;
        if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(Error(400, "Enter a device name of at most 80 bytes."));
        }
        let mut next = inner.disk.clone();
        let owner = bool::from(code.as_bytes().ct_eq(next.owner_key.as_bytes()));
        let (permissions, target_user) = if owner {
            (
                Permission::ALL.into_iter().collect(),
                Some(next.owner_user.clone()),
            )
        } else {
            if !hexadecimal(code, 32) {
                return Err(Error(401, "Invitation is invalid or expired."));
            }
            let digest = hash(code);
            let index = next
                .invitations
                .iter()
                .position(|i| {
                    bool::from(i.token_hash.as_bytes().ct_eq(digest.as_bytes()))
                        && i.expires_at_ms > now
                })
                .ok_or(Error(401, "Invitation is invalid or expired."))?;
            let invite = next.invitations.remove(index);
            let issuer = next.session(&invite.issuer, now).ok_or_else(forbidden)?;
            if !invite.permissions.is_subset(&issuer.device.permissions)
                || match &invite.target_user {
                    Some(user) => user != &issuer.user.id,
                    None => !issuer
                        .device
                        .permissions
                        .contains(&Permission::AccessManage),
                }
            {
                return Err(forbidden());
            }
            (invite.permissions, invite.target_user)
        };
        next.devices.retain(|d| d.device.expires_at_ms > now);
        if next.devices.len() >= 128 {
            return Err(Error(409, "Device limit reached."));
        }
        let user_id = match target_user {
            Some(id) => id,
            None => {
                if next.users.len() >= 1024 {
                    return Err(Error(409, "User limit reached."));
                }
                let user = User {
                    id: random(16)?,
                    name: name.trim().into(),
                };
                let id = user.id.clone();
                next.users.push(user);
                id
            }
        };
        let token = random(32)?;
        let device = Device {
            user_id,
            id: random(8)?,
            name: name.trim().into(),
            permissions,
            expires_at_ms: now.saturating_add(SESSION_MS),
        };
        next.devices.push(StoredDevice {
            device: device.clone(),
            token_hash: hash(&token),
        });
        persist(&self.path, &next)?;
        let session = next.session(&device.id, now).ok_or_else(unavailable)?;
        inner.disk = next;
        Ok(PairResponse {
            session,
            token: Some(token),
        })
    }
    pub fn devices(&self, actor: &Session, now: u64) -> Result<Vec<Device>> {
        let inner = self.inner.lock().map_err(|_| unavailable())?;
        inner.disk.administrator(actor, now)?;
        Ok(inner
            .disk
            .devices
            .iter()
            .filter(|d| d.device.expires_at_ms > now)
            .map(|d| d.device.clone())
            .collect())
    }
    pub fn revoke(&self, actor: &Session, id: &str, now: u64) -> Result<()> {
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        if id == "owner" {
            return Err(Error(
                403,
                "Local owner authority cannot be revoked remotely.",
            ));
        }
        let actor = inner.disk.current(actor, now)?;
        let own = inner
            .disk
            .devices
            .iter()
            .any(|d| d.device.id == id && d.device.user_id == actor.user.id);
        if !own {
            let administrator = inner.disk.administrator(&actor, now)?;
            if inner
                .disk
                .devices
                .iter()
                .find(|d| d.device.id == id)
                .is_some_and(|d| {
                    !d.device
                        .permissions
                        .is_subset(&administrator.device.permissions)
                })
            {
                return Err(forbidden());
            }
        }
        let mut next = inner.disk.clone();
        next.devices.retain(|d| d.device.id != id);
        next.invitations.retain(|i| i.issuer != id);
        persist(&self.path, &next)?;
        inner.disk = next;
        Ok(())
    }
}
fn persist(path: &std::path::Path, disk: &Disk) -> Result<()> {
    let bytes = serde_json::to_vec(disk).map_err(|_| unavailable())?;
    if bytes.len() > 1024 * 1024 {
        return Err(unavailable());
    }
    let temporary = path.with_extension(format!("{}.tmp", random(8)?));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| unavailable())?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|_| unavailable())
}
