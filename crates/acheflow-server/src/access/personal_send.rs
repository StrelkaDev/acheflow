//! One-shot personal sends. Never registers a collector or borrows a shared account.
use super::registry::StoredPersonalAccount;
use super::*;
use acheflow_client::{
    runtime::{Identity, ProviderRuntime},
    server_account::ServerAccount,
};
use acheflow_extension_api::{client_execution::*, integration::valid_id};
use acheflow_media_api::personal_account::*;
use std::collections::BTreeMap;
struct Entry {
    digest: String,
    at: u64,
    outcome: ExecutionOutcome,
}
pub(super) struct Ledger {
    pub epoch: String,
    entries: BTreeMap<(String, String, String), Entry>,
}
impl Ledger {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            epoch: registry::random(16)?,
            entries: BTreeMap::new(),
        })
    }
    fn reserve(
        &mut self,
        user: &str,
        workspace: &str,
        digest: &str,
        r: &SendPersonalChat,
        time: u64,
    ) -> Result<Option<ExecutionOutcome>, Error> {
        if r.epoch != self.epoch || !valid_id(&r.request_id, 128) {
            return Err(Error(
                409,
                "Send session expired. Check delivery before composing a new message.",
            ));
        }
        self.entries
            .retain(|_, e| e.at.saturating_add(INTENT_RETENTION_MS) > time);
        let key = (user.into(), workspace.into(), r.request_id.clone());
        if let Some(old) = self.entries.get(&key) {
            return if old.digest == digest {
                Ok(Some(old.outcome))
            } else {
                Err(Error(409, "Message ID was reused for another request."))
            };
        }
        if r.issued_at_ms > time.saturating_add(5000)
            || time > r.issued_at_ms.saturating_add(30_000)
        {
            return Err(Error(
                409,
                "Message request expired. Refresh your account status.",
            ));
        }
        if self.entries.len() >= 512 {
            return Err(Error(429, "Personal message ledger is full. Try later."));
        }
        self.entries.insert(
            key,
            Entry {
                digest: digest.into(),
                at: time,
                outcome: ExecutionOutcome::Unknown,
            },
        );
        Ok(None)
    }
}
fn check(
    access: &Access,
    actor: &Principal,
    workspace: &str,
    provider: &str,
    channel: &str,
    version: &str,
) -> Result<StoredPersonalAccount, Error> {
    let actor = access.registry.current(&actor.0, now())?;
    if access.host.runtime.shutdown.is_cancelled() {
        return Err(Error(409, "Server is shutting down."));
    }
    if !actor.device.permissions.contains(&P::ChatSend) {
        return Err(denied());
    }
    if !valid_id(channel, 128) {
        return Err(Error(422, "Invalid channel."));
    }
    let current = access.host.runtime.workspace.read().unwrap();
    if !current.as_ref().is_some_and(|w| w.id == workspace)
        || !access
            .host
            .runtime
            .activated
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(Error(409, "The server workspace is not active."));
    }
    let configured = current
        .as_ref()
        .and_then(|w| w.server_integrations.get(provider))
        .is_some_and(|b| b.enabled && b.channel_id == channel);
    drop(current);
    if !configured
        && !access
            .host
            .snapshot(0, 0)
            .channels
            .iter()
            .any(|c| c.extension_id == provider && c.id == channel)
    {
        return Err(Error(
            403,
            "This channel is not connected to the workspace.",
        ));
    }
    let binding = access
        .registry
        .personal_account(&actor, workspace, provider, now())?
        .ok_or(Error(
            403,
            "Connect your own account for this provider first.",
        ))?;
    if binding.version != version {
        return Err(Error(
            409,
            "Your personal account changed. Refresh before sending.",
        ));
    }
    Ok(binding)
}
trait Engine {
    type Ready;
    async fn prepare(
        &self,
        access: &Access,
        b: &StoredPersonalAccount,
        channel: &str,
    ) -> Result<Self::Ready, String>;
    async fn execute(&self, ready: Self::Ready, id: &str, text: &str) -> ExecutionOutcome;
}
struct RuntimeEngine;
struct Ready {
    account: ServerAccount,
    runtime: ProviderRuntime,
}
impl Engine for RuntimeEngine {
    type Ready = Ready;
    async fn prepare(
        &self,
        access: &Access,
        b: &StoredPersonalAccount,
        channel: &str,
    ) -> Result<Ready, String> {
        let installed = access
            .host
            .provider_catalog
            .installed
            .get(&b.provider)
            .ok_or("Provider unavailable")?;
        let identity = Identity {
            workspace_id: b.workspace.clone(),
            client_id: format!("personal-{}", b.user),
            generation: 1,
            connection: Connection {
                id: registry::random(16).map_err(|_| "Random generator unavailable")?,
                provider: b.provider.clone(),
                account_id: b.account_id.clone(),
                channel_id: channel.into(),
                capabilities: [Capability::ChatSend].into(),
            },
        };
        let (account, mut runtime) = ServerAccount::personal_runtime(
            &access.registry.server_id(),
            &b.workspace,
            &b.user,
            &b.slot,
            installed,
            b.config.clone(),
            identity,
        )
        .await?;
        if !runtime.prepare_action(now()).await? {
            return Err("Platform does not permit sending here".into());
        }
        Ok(Ready { account, runtime })
    }
    async fn execute(&self, mut ready: Ready, id: &str, text: &str) -> ExecutionOutcome {
        if ready.account.install(&mut ready.runtime).is_err() {
            return ExecutionOutcome::Rejected;
        }
        let time = now();
        let identity = ready.runtime.identity();
        let claim = ClaimedInstruction {
            lease: 1,
            instruction: Instruction {
                connection: identity.connection.clone(),
                id: id.into(),
                request_id: Some(id.into()),
                workspace_id: identity.workspace_id.clone(),
                client_id: identity.client_id.clone(),
                generation: identity.generation,
                connection_id: identity.connection.id.clone(),
                action: ClientAction::SendChat { text: text.into() },
                created_at_ms: time,
                expires_at_ms: time + 10_000,
                state: InstructionState::InFlight,
                external_id: None,
            },
        };
        let outcome = ready
            .runtime
            .execute(claim, time)
            .await
            .map(|(r, _)| r.outcome)
            .unwrap_or(ExecutionOutcome::Unknown);
        if ready.account.checkpoint(&ready.runtime).is_err() {
            eprintln!("Personal send credential checkpoint failed");
        }
        outcome
    }
}
pub(super) async fn send(
    State(access): State<Arc<Access>>,
    Extension(actor): Extension<Principal>,
    Path((workspace, provider, channel)): Path<(String, String, String)>,
    Json(request): Json<SendPersonalChat>,
) -> Result<Json<PersonalDelivery>, Error> {
    send_with(
        &access,
        &actor,
        (&workspace, &provider, &channel),
        request,
        &RuntimeEngine,
    )
    .await
    .map(Json)
}
async fn send_with(
    access: &Access,
    actor: &Principal,
    target: (&str, &str, &str),
    request: SendPersonalChat,
    engine: &impl Engine,
) -> Result<PersonalDelivery, Error> {
    let (workspace, provider, channel) = target;
    if request.text.trim().is_empty() || request.text.len() > 8192 {
        return Err(Error(422, "Enter a message of at most 8192 bytes."));
    }
    let binding = check(
        access,
        actor,
        workspace,
        provider,
        channel,
        &request.expected_version,
    )?;
    let digest = registry::hash(
        &serde_json::to_string(&(provider, channel, &request))
            .map_err(|_| Error(422, "Invalid message"))?,
    );
    let _permit = access
        .account_checks
        .try_acquire()
        .map_err(|_| Error(429, "Another account operation is in progress."))?;
    if let Some(outcome) = access.personal_sends.lock().unwrap().reserve(
        &binding.user,
        workspace,
        &digest,
        &request,
        now(),
    )? {
        return Ok(PersonalDelivery { outcome });
    }
    let prepared = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        engine.prepare(access, &binding, channel),
    )
    .await;
    let outcome = match prepared {
        Ok(Ok(ready))
            if check(
                access,
                actor,
                workspace,
                provider,
                channel,
                &request.expected_version,
            )
            .is_ok() =>
        {
            tokio::time::timeout(
                std::time::Duration::from_secs(12),
                engine.execute(ready, &request.request_id, &request.text),
            )
            .await
            .unwrap_or(ExecutionOutcome::Unknown)
        }
        _ => ExecutionOutcome::Rejected,
    };
    if let Some(entry) = access.personal_sends.lock().unwrap().entries.get_mut(&(
        binding.user.clone(),
        workspace.into(),
        request.request_id.clone(),
    )) {
        entry.outcome = outcome;
    }
    eprintln!(
        "Personal chat send user={} device={} provider={} channel={} request={} outcome={:?}",
        binding.user, actor.0.device.id, provider, channel, request.request_id, outcome
    );
    Ok(PersonalDelivery { outcome })
}
#[cfg(test)]
mod tests;
