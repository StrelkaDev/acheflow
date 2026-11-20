//! Explicit trusted-server connection. No caller-supplied storage or account identity is trusted.
use super::*;
use acheflow_client::server_account::{ServerAccount, authorization};
use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
use acheflow_media_api::{server_provider::*, workspace::Workspace};

fn check(
    access: &Access,
    actor: &Principal,
    workspace: &str,
    revision: u64,
) -> Result<Workspace, Error> {
    let current = Principal(access.registry.current(&actor.0, now())?);
    if !current.server_accounts() {
        return Err(denied());
    }
    let (actual, saved) = access.host.runtime.saved_state();
    if actual != revision {
        return Err(Error(
            409,
            "Workspace changed. Refresh before connecting an account.",
        ));
    }
    saved.filter(|w| w.id == workspace).ok_or(Error(
        409,
        "Select the server workspace before connecting an account.",
    ))
}
pub(super) async fn connect(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(provider): Path<String>,
    Json(request): Json<ConnectServerAccount>,
) -> Result<Json<ServerAccountConnected>, Error> {
    if !access.host.package_execution_allowed(&provider) {
        return Err(Error(
            403,
            "Approve this server package before authorizing it.",
        ));
    }
    if !request.trust_server {
        return Err(Error(
            400,
            "Confirm that you trust this server with the account.",
        ));
    }
    check(
        &access,
        &principal,
        &request.workspace_id,
        request.expected_revision,
    )?;
    let installed = access
        .host
        .provider(&provider)
        .ok_or(Error(404, "Provider is not installed on this server."))?;
    if request.config.len() > 32
        || request.config.iter().any(|(key, value)| {
            installed
                .manifest
                .settings
                .iter()
                .find(|s| s.id == *key)
                .is_none_or(|s| {
                    s.secret || value.len() > s.max_length || value.chars().any(char::is_control)
                })
        })
        || request.expected_account.is_empty()
        || request.expected_account.len() > 128
    {
        return Err(Error(422, "Invalid account settings."));
    }
    // Validate all public fields before making any external request or OS-vault write.
    let mut proposed = check(
        &access,
        &principal,
        &request.workspace_id,
        request.expected_revision,
    )?;
    let binding = ServerIntegration {
        enabled: true,
        application: registry::random(16)?,
        account_id: request.expected_account.clone(),
        channel_id: request.channel_id.clone(),
        config: request.config.clone(),
    };
    prepare(&mut proposed, &provider, &binding);
    crate::live::validate(&proposed).map_err(|_| Error(422, "Invalid server channel settings."))?;
    crate::workspace_extensions::validate_public_settings(&access.host, &proposed)
        .map_err(|_| Error(422, "Invalid server provider settings."))?;
    let verified = verify(
        &access,
        &provider,
        request.config,
        request.credentials,
        &request.expected_account,
    )
    .await?;
    complete(
        &access,
        &principal,
        PendingAccount {
            workspace: request.workspace_id,
            revision: request.expected_revision,
            provider,
            binding,
        },
        verified,
        &SystemStore,
    )
    .await
}
pub(super) async fn verify(
    access: &Access,
    provider: &str,
    config: std::collections::BTreeMap<String, String>,
    credentials: std::collections::BTreeMap<String, String>,
    expected_account: &str,
) -> Result<authorization::AuthorizedAccount, Error> {
    let installed = access
        .host
        .provider(provider)
        .ok_or(Error(404, "Provider is not installed on this server."))?;
    if config.len() > 32
        || config.iter().any(|(key, value)| {
            installed
                .manifest
                .settings
                .iter()
                .find(|s| s.id == *key)
                .is_none_or(|s| {
                    s.secret || value.len() > s.max_length || value.chars().any(char::is_control)
                })
        })
        || !acheflow_extension_api::integration::valid_id(expected_account, 128)
    {
        return Err(Error(422, "Invalid account settings."));
    }
    let _validation = access
        .account_checks
        .try_acquire()
        .map_err(|_| Error(429, "Another account is being checked. Try again shortly."))?;
    let verified = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        authorization::authorize(
            installed.manifest.clone(),
            &installed.module,
            config,
            credentials,
        ),
    )
    .await
    .map_err(|_| Error(504, "Account validation timed out."))?
    .map_err(|_| {
        Error(
            422,
            "The server could not verify this account. Reauthorize it and check its permissions.",
        )
    })?;
    if verified.account_id() != expected_account {
        return Err(Error(
            422,
            "The platform confirmed a different account. Reauthorize and try again.",
        ));
    }
    Ok(verified)
}
struct PendingAccount {
    workspace: String,
    revision: u64,
    provider: String,
    binding: ServerIntegration,
}
trait AccountStore: Sync {
    fn save(
        &self,
        account: &authorization::AuthorizedAccount,
        server: &str,
        pending: &PendingAccount,
    ) -> Result<(), String>;
    fn remove(
        &self,
        server: &str,
        workspace: &str,
        provider: &str,
        binding: &ServerIntegration,
    ) -> Result<(), String>;
}
struct SystemStore;
impl AccountStore for SystemStore {
    fn save(
        &self,
        account: &authorization::AuthorizedAccount,
        server: &str,
        pending: &PendingAccount,
    ) -> Result<(), String> {
        account.save(
            server,
            &pending.workspace,
            &pending.provider,
            &pending.binding.application,
        )
    }
    fn remove(
        &self,
        server: &str,
        workspace: &str,
        provider: &str,
        binding: &ServerIntegration,
    ) -> Result<(), String> {
        ServerAccount::remove(
            server,
            workspace,
            provider,
            &binding.application,
            &binding.account_id,
        )
    }
}
async fn complete(
    access: &Access,
    principal: &Principal,
    pending: PendingAccount,
    verified: authorization::AuthorizedAccount,
    store: &impl AccountStore,
) -> Result<Json<ServerAccountConnected>, Error> {
    let _command = access.host.command_lock.lock().await;
    let previous = check(access, principal, &pending.workspace, pending.revision)?;
    if verified.account_id() != pending.binding.account_id {
        return Err(Error(
            422,
            "The verified account does not match this binding.",
        ));
    }
    let server = access.registry.server_id();
    store.save(&verified, &server, &pending)
        .map_err(|_| Error(503, "Cannot save the account in the server credential store. The local account is unchanged."))?;
    let saved = access
        .host
        .runtime
        .update_workspace(&pending.workspace, |workspace| {
            if workspace.edit_revision != pending.revision {
                return Err("conflict: workspace changed".into());
            }
            prepare(workspace, &pending.provider, &pending.binding);
            Ok(())
        });
    if saved.is_err() {
        if store
            .remove(
                &server,
                &pending.workspace,
                &pending.provider,
                &pending.binding,
            )
            .is_err()
        {
            eprintln!("Unused server account slot could not be removed");
        }
        return Err(Error(
            503,
            "Cannot save the server workspace. The previous account is unchanged.",
        ));
    }
    // Reconcile fences and joins the old collector before it can use new credentials.
    access.host.providers.reconcile(&access.host).await;
    if let Some(old) = previous.server_integrations.get(&pending.provider) {
        // Older slots are retained if the OS vault cannot remove them; never roll back a live replacement.
        if store
            .remove(&server, &pending.workspace, &pending.provider, old)
            .is_err()
        {
            eprintln!("Retired server account slot could not be removed");
        }
    }
    Ok(Json(ServerAccountConnected {
        revision: access.host.runtime.saved_state().0,
        binding: pending.binding,
    }))
}
fn prepare(workspace: &mut Workspace, provider: &str, binding: &ServerIntegration) {
    let old = workspace
        .server_integrations
        .insert(provider.into(), binding.clone());
    workspace.client_capabilities.retain(|g| {
        !(g.provider == provider
            && ((g.account_id == binding.account_id && g.channel_id == binding.channel_id)
                || old.as_ref().is_some_and(|old| {
                    g.account_id == old.account_id && g.channel_id == old.channel_id
                })))
    });
    workspace.client_capabilities.push(CapabilityGrant {
        provider: provider.into(),
        account_id: binding.account_id.clone(),
        channel_id: binding.channel_id.clone(),
        capabilities: [Capability::ChatRead, Capability::AudienceRead].into(),
    });
}
pub(super) async fn disconnect(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(provider): Path<String>,
    Json(request): Json<DisconnectServerAccount>,
) -> Result<StatusCode, Error> {
    disconnect_with(&access, &principal, &provider, request, &SystemStore).await
}
async fn disconnect_with(
    access: &Access,
    principal: &Principal,
    provider: &str,
    request: DisconnectServerAccount,
    store: &impl AccountStore,
) -> Result<StatusCode, Error> {
    let _command = access.host.command_lock.lock().await;
    let workspace = check(
        access,
        principal,
        &request.workspace_id,
        request.expected_revision,
    )?;
    let old = workspace
        .server_integrations
        .get(provider)
        .ok_or(Error(404, "Server account is not connected."))?;
    access
        .host
        .providers
        .stop_provider(&access.host, provider)
        .await;
    if let Err(error) = check(
        access,
        principal,
        &request.workspace_id,
        request.expected_revision,
    ) {
        access.host.providers.reconcile(&access.host).await;
        return Err(error);
    }
    let result = store.remove(&access.registry.server_id(), &workspace.id, provider, old);
    // Keep the server-owned binding reserved; disconnect must not restart a client's collector.
    access.host.providers.reconcile(&access.host).await;
    result.map_err(|_| {
        Error(
            503,
            "Cannot remove the account from the server credential store.",
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;

/// Only public declarations and approved WASM, never persisted account state.
pub(super) async fn catalog(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<acheflow_extension_api::integration::GuestManifest>>, Error> {
    let current = access.registry.current(&principal.0, now())?;
    if !current.device.permissions.contains(&P::ExtensionsConfigure) {
        return Err(denied());
    }
    Ok(Json(
        access
            .host
            .provider_catalog
            .read()
            .unwrap()
            .installed
            .values()
            .map(|p| p.manifest.clone())
            .collect(),
    ))
}
pub(super) async fn module(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(provider): Path<String>,
) -> Result<axum::response::Response, Error> {
    let current = Principal(access.registry.current(&principal.0, now())?);
    if !current.server_accounts() || !access.host.package_execution_allowed(&provider) {
        return Err(denied());
    }
    let installed = access
        .host
        .provider(&provider)
        .ok_or(Error(404, "Server integration is not installed."))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/wasm")],
        installed.module,
    )
        .into_response())
}

pub(super) async fn configure(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(provider): Path<String>,
    Json(request): Json<ConfigureServerIntegration>,
) -> Result<Json<ServerAccountConnected>, Error> {
    let _command = access.host.command_lock.lock().await;
    let current = Principal(access.registry.current(&principal.0, now())?);
    if !current.allows(P::ExtensionsConfigure) {
        return Err(denied());
    }
    let (revision, saved) = access.host.runtime.saved_state();
    if revision != request.expected_revision {
        return Err(Error(409, "Server settings changed."));
    }
    let mut workspace = saved
        .filter(|w| w.id == request.workspace_id)
        .ok_or(Error(409, "The server workspace changed."))?;
    let previous = workspace
        .server_integrations
        .get(&provider)
        .ok_or(Error(404, "Connect a server account first."))?;
    if previous.enabled != request.enabled && !current.allows(P::ExtensionsRun) {
        return Err(denied());
    }
    if request.enabled && !previous.enabled && !access.host.package_execution_allowed(&provider) {
        return Err(denied());
    }
    let binding = workspace
        .server_integrations
        .get_mut(&provider)
        .ok_or(Error(404, "Connect a server account first."))?;
    // Application identity chooses a credential slot and cannot be changed by a
    // settings write. Reauthorization is required for a different application.
    if binding.config.get("client_id") != request.config.get("client_id") {
        return Err(Error(
            409,
            "Reauthorize the server account after changing its application Client ID.",
        ));
    }
    binding.enabled = request.enabled;
    binding.config = request.config;
    let result = binding.clone();
    crate::providers::validate(&access.host, &workspace)
        .map_err(|_| Error(422, "Invalid server provider settings."))?;
    crate::live::validate(&workspace).map_err(|_| Error(422, "Invalid server settings."))?;
    access
        .host
        .runtime
        .update_workspace(&request.workspace_id, |w| {
            w.server_integrations
                .insert(provider.clone(), result.clone());
            Ok(())
        })
        .map_err(|_| Error(409, "Server settings changed."))?;
    access.host.providers.reconcile(&access.host).await;
    Ok(Json(ServerAccountConnected {
        revision: access.host.runtime.saved_state().0,
        binding: result,
    }))
}

pub(super) async fn settings(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Path(provider): Path<String>,
) -> Result<Json<ServerIntegrationSettings>, Error> {
    let current = access.registry.current(&principal.0, now())?;
    if !current.device.permissions.contains(&P::ExtensionsConfigure) {
        return Err(denied());
    }
    if access.host.provider(&provider).is_none() {
        return Err(Error(404, "Provider is not installed on this server."));
    }
    let (revision, workspace) = access.host.runtime.saved_state();
    Ok(Json(ServerIntegrationSettings {
        workspace_id: workspace.as_ref().map(|w| w.id.clone()),
        revision,
        binding: workspace.and_then(|w| w.server_integrations.get(&provider).cloned()),
    }))
}
