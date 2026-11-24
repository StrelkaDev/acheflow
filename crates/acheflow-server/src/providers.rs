//! One trusted executor per configured provider binding. No provider protocols here.
use crate::{client_bridge::ExecutorCredential, control::ControlHost};
use acheflow_client::{
    catalog::ClientCatalog,
    runtime::{Identity, ProviderRuntime},
    server_account::ServerAccount,
};
use acheflow_control::client_execution::channel_policy;
use acheflow_extension_api::client_execution::*;
use acheflow_media_api::{
    server_provider::{ServerIntegration, ServerProviderStatus},
    workspace::Workspace,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
mod reactor;
#[derive(Clone, PartialEq)]
struct Spec {
    workspace: String,
    binding: ServerIntegration,
    policy: Option<BTreeSet<Capability>>,
}
struct Entry {
    spec: Spec,
    actor: String,
    generation: u64,
    state: &'static str,
    registration: Option<ClientRegistration>,
}
struct Running {
    spec: Spec,
    actor: String,
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.join.abort();
    }
}
#[derive(Default)]
pub struct Manager {
    server: Mutex<Option<String>>,
    running: AsyncMutex<BTreeMap<String, Running>>,
    states: Mutex<BTreeMap<String, Entry>>,
}
impl Manager {
    pub fn set_server(&self, id: &str) -> Result<(), String> {
        let mut server = self.server.lock().unwrap();
        if server.as_deref().is_some_and(|old| old != id) {
            return Err("Provider executor belongs to another server.".into());
        }
        *server = Some(id.into());
        Ok(())
    }
    pub fn current(&self, workspace: &str, provider: &str, actor: &str) -> bool {
        self.states
            .lock()
            .unwrap()
            .get(provider)
            .is_some_and(|e| e.spec.workspace == workspace && e.actor == actor)
    }
    fn mark(&self, provider: &str, actor: &str, state: &'static str, generation: u64) {
        if let Some(entry) = self
            .states
            .lock()
            .unwrap()
            .get_mut(provider)
            .filter(|e| e.actor == actor)
        {
            entry.state = state;
            entry.generation = generation;
        }
    }
    fn lease(&self, provider: &str, actor: &str) -> Option<ClientRegistration> {
        self.states
            .lock()
            .unwrap()
            .get(provider)
            .filter(|e| e.actor == actor)
            .and_then(|e| e.registration.clone())
    }
    fn set_lease(&self, provider: &str, actor: &str, registration: Option<ClientRegistration>) {
        if let Some(entry) = self
            .states
            .lock()
            .unwrap()
            .get_mut(provider)
            .filter(|e| e.actor == actor)
        {
            entry.registration = registration;
        }
    }
    pub fn snapshot(
        &self,
        catalog: &ClientCatalog,
        workspace: Option<&Workspace>,
    ) -> Vec<ServerProviderStatus> {
        let states = self.states.lock().unwrap();
        let ids: BTreeSet<_> = catalog
            .installed
            .keys()
            .chain(
                workspace
                    .into_iter()
                    .flat_map(|w| w.server_integrations.keys()),
            )
            .collect();
        ids.into_iter()
            .map(|id| {
                let installed = catalog.installed.get(id);
                let binding = workspace.and_then(|w| w.server_integrations.get(id));
                let entry = states.get(id).filter(|e| {
                    workspace.is_some_and(|w| w.id == e.spec.workspace)
                        && binding == Some(&e.spec.binding)
                });
                let missing = installed.is_none() && binding.is_some_and(|b| b.enabled);
                ServerProviderStatus {
                    provider: id.clone(),
                    name: installed
                        .map(|i| i.manifest.name.clone())
                        .unwrap_or_else(|| id.clone()),
                    state: entry
                        .map(|e| e.state)
                        .unwrap_or(if missing { "failed" } else { "stopped" })
                        .into(),
                    generation: entry.map(|e| e.generation).unwrap_or(0),
                    server_owned: binding.is_some_and(|b| b.enabled),
                    account_id: binding.map(|b| b.account_id.clone()),
                    channel_id: binding.map(|b| b.channel_id.clone()),
                }
            })
            .collect()
    }
    pub async fn active(&self) -> bool {
        self.running
            .lock()
            .await
            .values()
            .any(|r| !r.join.is_finished())
    }
    pub async fn stop_provider(&self, host: &ControlHost, provider: &str) {
        let mut running = self.running.lock().await;
        if let Some(mut task) = running.remove(provider) {
            self.mark(provider, &task.actor, "stopped", 0);
            if let Some(entry) = self.states.lock().unwrap().get_mut(provider) {
                entry.actor.clear();
                entry.registration = None;
            }
            withdraw(host, &task.spec.workspace, &task.actor);
            task.cancel.cancel();
            let _ = (&mut task.join).await;
            host.client_execution.lock().unwrap().forget_retired(
                &task.spec.workspace,
                &task.actor,
                crate::control::now_ms(),
            );
        }
    }
    pub async fn suspend(&self, host: &ControlHost) {
        let mut running = self.running.lock().await;
        let old = std::mem::take(&mut *running);
        for (provider, mut task) in old {
            self.mark(&provider, &task.actor, "stopped", 0);
            if let Some(entry) = self.states.lock().unwrap().get_mut(&provider) {
                entry.actor.clear();
                entry.registration = None;
            }
            withdraw(host, &task.spec.workspace, &task.actor);
            host.client_execution.lock().unwrap().forget_retired(
                &task.spec.workspace,
                &task.actor,
                crate::control::now_ms(),
            );
            task.cancel.cancel();
            let _ = (&mut task.join).await;
        }
    }
    pub async fn reconcile(&self, host: &Arc<ControlHost>) {
        let workspace = host.runtime.workspace.read().unwrap().clone();
        if host.runtime.shutdown.is_cancelled() {
            self.suspend(host).await;
            return;
        }
        let Some(workspace) = workspace else {
            self.suspend(host).await;
            return;
        };
        let server = self.server.lock().unwrap().clone();
        if let Some(server) = server
            && let Err(error) =
                ServerAccount::name_workspace(&server, &workspace.id, &workspace.name)
        {
            eprintln!("Workspace credential label: {error}");
        }
        let desired: BTreeMap<_, _> = workspace
            .server_integrations
            .iter()
            .filter(|(_, b)| b.enabled)
            .map(|(id, b)| {
                (
                    id.clone(),
                    Spec {
                        workspace: workspace.id.clone(),
                        binding: b.clone(),
                        policy: channel_policy(
                            &workspace.client_capabilities,
                            id,
                            &b.account_id,
                            &b.channel_id,
                        ),
                    },
                )
            })
            .collect();
        let mut running = self.running.lock().await;
        let remove: Vec<_> = running
            .iter()
            .filter(|(id, r)| desired.get(*id) != Some(&r.spec) || r.join.is_finished())
            .map(|(id, _)| id.clone())
            .collect();
        for id in remove {
            if let Some(mut old) = running.remove(&id) {
                self.mark(&id, &old.actor, "stopped", 0);
                if let Some(entry) = self.states.lock().unwrap().get_mut(&id) {
                    entry.actor.clear();
                    entry.registration = None;
                }
                withdraw(host, &old.spec.workspace, &old.actor);
                host.client_execution.lock().unwrap().forget_retired(
                    &old.spec.workspace,
                    &old.actor,
                    crate::control::now_ms(),
                );
                old.cancel.cancel();
                let _ = (&mut old.join).await;
            }
        }
        for (provider, spec) in desired {
            if !host.package_execution_allowed(&provider) {
                continue;
            }
            if running.contains_key(&provider) {
                continue;
            }
            let Some(installed) = host.provider(&provider) else {
                continue;
            };
            let actor = format!(
                "server-{}",
                &acheflow_media_api::workspace::new_token()[..32]
            );
            self.states.lock().unwrap().insert(
                provider.clone(),
                Entry {
                    spec: spec.clone(),
                    actor: actor.clone(),
                    generation: 0,
                    state: "starting",
                    registration: None,
                },
            );
            retire_external(host, &workspace.id, &provider, &spec.binding.channel_id);
            if spec
                .policy
                .as_ref()
                .is_some_and(|policy| !policy.contains(&Capability::ChatRead))
            {
                self.mark(&provider, &actor, "not_authorized", 0);
                continue;
            }
            let Some(server) = self.server.lock().unwrap().clone() else {
                self.mark(&provider, &actor, "needs_authorization", 0);
                continue;
            };
            let cancel = host.runtime.shutdown.child_token();
            let join = tokio::spawn(worker(
                Arc::downgrade(host),
                provider.clone(),
                spec.clone(),
                actor.clone(),
                server,
                (installed.manifest.clone(), installed.module.clone()),
                cancel.clone(),
            ));
            running.insert(
                provider,
                Running {
                    spec,
                    actor,
                    cancel,
                    join,
                },
            );
        }
    }
}
fn withdraw(host: &ControlHost, workspace: &str, actor: &str) {
    let mut registry = host.client_execution.lock().unwrap();
    let now = crate::control::now_ms();
    for status in registry
        .connections(workspace, now)
        .into_iter()
        .filter(|s| s.client_id == actor)
    {
        let _ = registry.unregister(workspace, actor, status.generation, now);
    }
}
fn retire_external(host: &ControlHost, workspace: &str, provider: &str, channel: &str) {
    let mut registry = host.client_execution.lock().unwrap();
    let now = crate::control::now_ms();
    for status in registry.connections(workspace, now) {
        if status
            .connections
            .iter()
            .any(|c| c.provider == provider && c.channel_id == channel)
        {
            let _ = registry.unregister(workspace, &status.client_id, status.generation, now);
        }
    }
}
async fn worker(
    host: Weak<ControlHost>,
    provider: String,
    spec: Spec,
    actor: String,
    server: String,
    artifact: (acheflow_control::extensions::GuestManifest, Vec<u8>),
    cancel: CancellationToken,
) {
    let (manifest, module) = artifact;
    let mut generation = 1;
    let mut failures = 0u32;
    loop {
        let mark = |state, generation| {
            if let Some(host) = host.upgrade() {
                host.providers.mark(&provider, &actor, state, generation);
            }
        };
        let account = if manifest.secret_fields.is_empty() {
            Ok(None)
        } else {
            ServerAccount::load(
                &server,
                &spec.workspace,
                &provider,
                &spec.binding.application,
                &spec.binding.account_id,
            )
        };
        let mut account = match ready_account(account, manifest.secret_fields.is_empty()) {
            Ok(account) => account,
            Err(state) => {
                mark(state, generation);
                return;
            }
        };
        let identity = Identity {
            workspace_id: spec.workspace.clone(),
            client_id: actor.clone(),
            generation,
            connection: Connection {
                channel_label: None,
                stream_id: None,
                id: format!("binding-{generation}"),
                provider: provider.clone(),
                account_id: spec.binding.account_id.clone(),
                channel_id: spec.binding.channel_id.clone(),
                capabilities: BTreeSet::new(),
            },
        };
        let Ok(mut runtime) = ProviderRuntime::new_for_host(
            manifest.clone(),
            &module,
            identity,
            spec.binding.config.clone(),
            acheflow_extension_api::parts::ExecutionHost::Server,
        ) else {
            mark("failed", generation);
            return;
        };
        if account
            .as_ref()
            .is_some_and(|a| a.install(&mut runtime).is_err())
        {
            mark("needs_authorization", generation);
            return;
        }
        let bridge = reactor::Bridge {
            host: host.clone(),
            provider: provider.clone(),
            actor: ExecutorCredential {
                id: actor.clone(),
                workspace_id: spec.workspace.clone(),
                token: String::new(),
            },
        };
        let result = tokio::select! {biased; _=cancel.cancelled()=>None,result=reactor::run(&bridge,&mut runtime,&spec,&mut account)=>Some(result),result=reactor::heartbeat(&bridge,std::time::Duration::from_secs(5))=>Some(result)};
        generation = runtime.identity().generation.saturating_add(1);
        drop(runtime);
        drop(account);
        if let Some(host) = host.upgrade() {
            host.providers.set_lease(&provider, &actor, None);
            withdraw(&host, &spec.workspace, &actor);
        } else {
            return;
        }
        if result.is_none() {
            mark("stopped", 0);
            return;
        }
        let terminal = match result.as_ref() {
            Some(Err(error)) if error == "account_storage_unavailable" => {
                Some("storage_unavailable")
            }
            Some(Err(error)) if error == "provider_needs_authorization" => {
                Some("needs_authorization")
            }
            _ => None,
        };
        if let Some(state) = terminal {
            mark(state, generation);
            return;
        }

        failures = failures.saturating_add(1);
        mark("reconnecting", generation);
        tokio::select! {biased; _=cancel.cancelled()=>return,_=tokio::time::sleep(std::time::Duration::from_secs((1u64<<failures.min(5)).min(30)))=>{}}
    }
}
fn ready_account<A>(
    loaded: Result<Option<A>, String>,
    secretless: bool,
) -> Result<Option<A>, &'static str> {
    match loaded {
        Ok(Some(account)) => Ok(Some(account)),
        Ok(None) if secretless => Ok(None),
        Ok(None) => Err("needs_authorization"),
        Err(_) => Err("storage_unavailable"),
    }
}

pub(crate) fn validate(host: &ControlHost, workspace: &Workspace) -> Result<(), String> {
    for (provider, binding) in &workspace.server_integrations {
        if host.installed.read().unwrap().contains_key(provider) {
            return Err("Provider identity conflicts with server logic.".into());
        }
        let installed = host
            .provider(provider)
            .ok_or("Unknown server provider installation.")?;
        if binding.config.len() > 32
            || binding.config.iter().any(|(key, value)| {
                installed
                    .manifest
                    .settings
                    .iter()
                    .find(|s| s.id == *key)
                    .is_none_or(|s| {
                        s.secret
                            || value.len() > s.max_length
                            || value.chars().any(char::is_control)
                    })
            })
        {
            return Err("Invalid public server provider settings.".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
