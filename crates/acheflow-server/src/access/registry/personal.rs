use super::*;
use acheflow_extension_api::integration::{valid_id, valid_provider_id};
use acheflow_media_api::personal_account::PersonalAccount;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::access) struct StoredPersonalAccount {
    pub workspace: String,
    pub user: String,
    pub provider: String,
    pub account_id: String,
    pub slot: String,
    pub version: String,
    pub config: std::collections::BTreeMap<String, String>,
}
impl StoredPersonalAccount {
    pub fn public(&self) -> PersonalAccount {
        PersonalAccount {
            provider: self.provider.clone(),
            account_id: self.account_id.clone(),
            version: self.version.clone(),
        }
    }
}
impl Disk {
    pub(super) fn validate_personal(&self) -> Result<()> {
        let mut bindings = BTreeSet::new();
        let mut slots = BTreeSet::new();
        if self.personal_accounts.len() > 256
            || self.personal_accounts.iter().any(|a| {
                !self.users.iter().any(|u| u.id == a.user)
                    || !valid_id(&a.workspace, 128)
                    || !valid_provider_id(&a.provider)
                    || !valid_id(&a.account_id, 128)
                    || !hexadecimal(&a.slot, 32)
                    || !hexadecimal(&a.version, 32)
                    || a.config.len() > 32
                    || a.config.iter().any(|(k, v)| {
                        !valid_id(k, 64) || v.len() > 8192 || v.chars().any(char::is_control)
                    })
                    || !bindings.insert((&a.workspace, &a.user, &a.provider))
                    || !slots.insert(&a.slot)
            })
        {
            return Err(unavailable());
        }
        Ok(())
    }
}
impl Registry {
    pub fn personal_accounts(
        &self,
        actor: &Session,
        workspace: &str,
        now: u64,
    ) -> Result<Vec<PersonalAccount>> {
        let inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        Ok(inner
            .disk
            .personal_accounts
            .iter()
            .filter(|a| a.user == actor.user.id && a.workspace == workspace)
            .map(StoredPersonalAccount::public)
            .collect())
    }
    pub fn personal_account(
        &self,
        actor: &Session,
        workspace: &str,
        provider: &str,
        now: u64,
    ) -> Result<Option<StoredPersonalAccount>> {
        let inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        Ok(inner
            .disk
            .personal_accounts
            .iter()
            .find(|a| a.user == actor.user.id && a.workspace == workspace && a.provider == provider)
            .cloned())
    }
    pub fn set_personal(
        &self,
        actor: &Session,
        account: StoredPersonalAccount,
        expected: Option<&str>,
        now: u64,
    ) -> Result<Option<StoredPersonalAccount>> {
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        if account.user != actor.user.id {
            return Err(forbidden());
        }
        let old = inner
            .disk
            .personal_accounts
            .iter()
            .find(|a| {
                a.user == actor.user.id
                    && a.workspace == account.workspace
                    && a.provider == account.provider
            })
            .cloned();
        if old.as_ref().map(|a| a.version.as_str()) != expected {
            return Err(Error(
                409,
                "Personal account changed. Refresh and try again.",
            ));
        }
        let mut next = inner.disk.clone();
        next.personal_accounts.retain(|a| {
            !(a.user == account.user
                && a.workspace == account.workspace
                && a.provider == account.provider)
        });
        next.personal_accounts.push(account);
        next.validate_personal()?;
        persist(&self.path, &next)?;
        inner.disk = next;
        Ok(old)
    }
    pub fn remove_personal(
        &self,
        actor: &Session,
        workspace: &str,
        provider: &str,
        expected: &str,
        now: u64,
    ) -> Result<StoredPersonalAccount> {
        let mut inner = self.inner.lock().map_err(|_| unavailable())?;
        let actor = inner.disk.current(actor, now)?;
        let old = inner
            .disk
            .personal_accounts
            .iter()
            .find(|a| a.user == actor.user.id && a.workspace == workspace && a.provider == provider)
            .ok_or(Error(404, "Personal account is not connected."))?
            .clone();
        if old.version != expected {
            return Err(Error(
                409,
                "Personal account changed. Refresh and try again.",
            ));
        }
        let mut next = inner.disk.clone();
        next.personal_accounts.retain(|a| a.slot != old.slot);
        persist(&self.path, &next)?;
        inner.disk = next;
        Ok(old)
    }
}
