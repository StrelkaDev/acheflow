use super::*;

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 80 && !name.chars().any(char::is_control)
}
impl Disk {
    pub(super) fn validate_users(&self) -> Result<()> {
        if self.version == 1 {
            if !self.personal_accounts.is_empty()
                || !self.users.is_empty()
                || !self.owner_user.is_empty()
                || self.devices.iter().any(|d| !d.device.user_id.is_empty())
                || self.invitations.iter().any(|i| i.target_user.is_some())
            {
                return Err(unavailable());
            }
            return Ok(());
        }
        let mut ids = BTreeSet::new();
        if self.users.len() > 1024
            || self.users.iter().any(|u| {
                !hexadecimal(&u.id, 32) || !valid_name(&u.name) || !ids.insert(u.id.as_str())
            })
            || !ids.contains(self.owner_user.as_str())
            || self
                .devices
                .iter()
                .any(|d| !ids.contains(d.device.user_id.as_str()))
            || self.invitations.iter().any(|i| {
                i.target_user
                    .as_ref()
                    .is_some_and(|id| !ids.contains(id.as_str()))
            })
        {
            return Err(unavailable());
        }
        Ok(())
    }
    pub(super) fn migrate_users(&mut self) -> Result<()> {
        let owner = User {
            id: random(16)?,
            name: "Server owner".into(),
        };
        self.owner_user = owner.id.clone();
        self.users.push(owner);
        // Legacy devices have no evidence of shared human ownership.
        for stored in &mut self.devices {
            let user = User {
                id: random(16)?,
                name: stored.device.name.clone(),
            };
            if !valid_name(&user.name) {
                return Err(unavailable());
            }
            stored.device.user_id = user.id.clone();
            self.users.push(user);
        }
        self.version = 2;
        self.validate_users()
    }
}
impl Registry {
    pub fn my_devices(&self, actor: &Session, now: u64) -> Result<Vec<Device>> {
        let inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        Ok(inner
            .disk
            .devices
            .iter()
            .filter(|d| d.device.expires_at_ms > now && d.device.user_id == actor.user.id)
            .map(|d| d.device.clone())
            .collect())
    }
    pub fn profile(&self, actor: &Session, name: &str, now: u64) -> Result<Session> {
        if !valid_name(name) {
            return Err(Error(400, "Enter a name of at most 80 bytes."));
        }
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        let mut next = inner.disk.clone();
        let user = next
            .users
            .iter_mut()
            .find(|u| u.id == actor.user.id)
            .ok_or_else(forbidden)?;
        user.name = name.trim().into();
        let session = next.session(&actor.device.id, now).ok_or_else(forbidden)?;
        persist(&self.path, &next)?;
        inner.disk = next;
        Ok(session)
    }
}
