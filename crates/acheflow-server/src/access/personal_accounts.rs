//! Read/delete-only migration for accounts saved by earlier server versions.
use super::registry::StoredPersonalAccount;
use super::*;
use acheflow_client::server_account::ServerAccount;
use acheflow_media_api::personal_account::*;
pub(super) async fn list(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(workspace): Path<String>,
) -> Result<Json<PersonalAccounts>, Error> {
    Ok(Json(PersonalAccounts {
        accounts: access
            .registry
            .personal_accounts(&principal.0, &workspace, now())?,
    }))
}
trait Store: Sync {
    fn remove(&self, server: &str, binding: &StoredPersonalAccount) -> Result<(), String>;
}
struct SystemStore;
impl Store for SystemStore {
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
pub(super) async fn retired() -> Error {
    Error(
        410,
        "Personal accounts and actions now stay on the originating device. Reauthorize in Extensions → Client. Existing server copies can still be listed and disconnected.",
    )
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
    // Upload and execution are retired. Retain cleanup metadata while a locked
    // vault prevents deletion so the user can unlock and retry explicitly.
    let old = access
        .registry
        .legacy_personal_account(&principal.0, workspace, provider, now())?;
    if old.version != expected {
        return Err(Error(
            409,
            "Personal account changed. Refresh and try again.",
        ));
    }
    store
        .remove(&access.registry.server_id(), &old)
        .map_err(|_| {
            Error(
                503,
                "Unlock the server credential store and retry removing the old copy.",
            )
        })?;
    access
        .registry
        .remove_personal(&principal.0, workspace, provider, expected, now())?;
    let credentials_removed = true;
    Ok(PersonalAccountDisconnected {
        credentials_removed,
    })
}
#[cfg(test)]
mod tests;
