//! Personal ownership is derived exclusively from the authenticated server user.
use super::registry::StoredPersonalAccount;
use super::*;
use acheflow_client::server_account::{ServerAccount, authorization::AuthorizedAccount};
use acheflow_media_api::personal_account::*;

fn current(
    access: &Access,
    principal: &Principal,
) -> Result<acheflow_media_api::access::Session, Error> {
    access.registry.current(&principal.0, now())
}
fn workspace(access: &Access, id: &str) -> Result<(), Error> {
    if access
        .host
        .runtime
        .workspace
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|w| w.id == id)
    {
        Ok(())
    } else {
        Err(Error(
            409,
            "Open the shared server workspace before connecting a personal account.",
        ))
    }
}
pub(super) async fn list(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(workspace): Path<String>,
) -> Result<Json<PersonalAccounts>, Error> {
    Ok(Json(PersonalAccounts {
        send_epoch: access.personal_sends.lock().unwrap().epoch.clone(),
        server_time_ms: now(),
        accounts: access
            .registry
            .personal_accounts(&principal.0, &workspace, now())?,
    }))
}
pub(super) async fn connect(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path((workspace_id, provider)): Path<(String, String)>,
    Json(request): Json<ConnectPersonalAccount>,
) -> Result<Json<PersonalAccount>, Error> {
    if !request.trust_server {
        return Err(Error(
            400,
            "Confirm that you trust this server with your account.",
        ));
    }
    let actor = current(&access, &principal)?;
    workspace(&access, &workspace_id)?;
    let previous = access
        .registry
        .personal_account(&actor, &workspace_id, &provider, now())?;
    if previous.as_ref().map(|a| a.version.as_str()) != request.expected_version.as_deref() {
        return Err(Error(
            409,
            "Personal account changed. Refresh and try again.",
        ));
    }
    let verified = server_accounts::verify(
        &access,
        &provider,
        request.config.clone(),
        request.credentials,
        &request.expected_account,
    )
    .await?;
    let pending = StoredPersonalAccount {
        workspace: workspace_id,
        user: actor.user.id,
        provider,
        account_id: verified.account_id().into(),
        slot: registry::random(16)?,
        version: registry::random(16)?,
        config: request.config,
    };
    complete(
        &access,
        &principal,
        pending,
        request.expected_version.as_deref(),
        verified,
        &SystemStore,
    )
    .await
    .map(Json)
}
trait Store: Sync {
    fn save(
        &self,
        account: &AuthorizedAccount,
        server: &str,
        binding: &StoredPersonalAccount,
    ) -> Result<(), String>;
    fn remove(&self, server: &str, binding: &StoredPersonalAccount) -> Result<(), String>;
}
struct SystemStore;
impl Store for SystemStore {
    fn save(
        &self,
        account: &AuthorizedAccount,
        server: &str,
        b: &StoredPersonalAccount,
    ) -> Result<(), String> {
        account.save_personal(server, &b.workspace, &b.user, &b.provider, &b.slot)
    }
    fn remove(&self, server: &str, b: &StoredPersonalAccount) -> Result<(), String> {
        ServerAccount::remove_personal(
            server,
            &b.workspace,
            &b.user,
            &b.provider,
            &b.slot,
            &b.account_id,
        )
    }
}
async fn complete(
    access: &Access,
    principal: &Principal,
    pending: StoredPersonalAccount,
    expected: Option<&str>,
    verified: AuthorizedAccount,
    store: &impl Store,
) -> Result<PersonalAccount, Error> {
    let _command = access.host.command_lock.lock().await;
    let actor = current(access, principal)?;
    workspace(access, &pending.workspace)?;
    if actor.user.id != pending.user || verified.account_id() != pending.account_id {
        return Err(denied());
    }
    let old =
        access
            .registry
            .personal_account(&actor, &pending.workspace, &pending.provider, now())?;
    if old.as_ref().map(|a| a.version.as_str()) != expected {
        return Err(Error(
            409,
            "Personal account changed. Refresh and try again.",
        ));
    }
    let server = access.registry.server_id();
    store.save(&verified, &server, &pending).map_err(|_| {
        Error(
            503,
            "Cannot save your personal account in the server credential store.",
        )
    })?;
    if let Err(error) = access
        .registry
        .set_personal(&actor, pending.clone(), expected, now())
    {
        if store.remove(&server, &pending).is_err() {
            eprintln!("Unused personal account slot could not be removed");
        }
        return Err(error);
    }
    if let Some(old) = old
        && store.remove(&server, &old).is_err()
    {
        eprintln!("Retired personal account slot could not be removed");
    }
    // No shared workspace mutation, collector reconciliation, channel registration or policy grant.
    Ok(pending.public())
}
pub(super) async fn disconnect(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path((workspace, provider)): Path<(String, String)>,
    Json(request): Json<DisconnectPersonalAccount>,
) -> Result<Json<PersonalAccountDisconnected>, Error> {
    disconnect_with(
        &access,
        &principal,
        &workspace,
        &provider,
        &request.expected_version,
        &SystemStore,
    )
    .await
    .map(Json)
}
async fn disconnect_with(
    access: &Access,
    principal: &Principal,
    workspace: &str,
    provider: &str,
    expected: &str,
    store: &impl Store,
) -> Result<PersonalAccountDisconnected, Error> {
    let _command = access.host.command_lock.lock().await;
    // Revoke the durable binding before cleanup, even if the OS vault is locked.
    let old =
        access
            .registry
            .remove_personal(&principal.0, workspace, provider, expected, now())?;
    let credentials_removed = store.remove(&access.registry.server_id(), &old).is_ok();
    if !credentials_removed {
        eprintln!("Disconnected personal account slot could not be removed");
    }
    Ok(PersonalAccountDisconnected {
        credentials_removed,
    })
}
#[cfg(test)]
mod tests;
