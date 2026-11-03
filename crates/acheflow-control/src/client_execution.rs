//! Pure registry/outbox. The caller supplies authenticated actor IDs and server time.
//! No platform requests, credentials, MCP transport or executable payloads live here.
use acheflow_extension_api::{client_execution::*, integration::valid_id};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CLIENTS: usize = 64;
const MAX_CONNECTIONS: usize = 16;
const MAX_WORKSPACES: usize = 64;
const MAX_INSTRUCTIONS: usize = 512;

struct Registration {
    requested: ClientRegistration,
    status: ExecutorStatus,
    active: bool,
}
struct Pending {
    instruction: Instruction,
    order: u64,
    lease: Option<u64>,
    receipt: Option<ExecutionReceipt>,
}
#[derive(Default)]
pub struct ExecutionRegistry {
    policies: BTreeMap<String, Vec<CapabilityGrant>>,
    clients: BTreeMap<(String, String), Registration>,
    instructions: BTreeMap<(String, String), Pending>,
    next_lease: u64,
    next_order: u64,
}

fn identity(value: &str) -> bool {
    valid_id(value, 128)
}
fn granted(connection: &Connection, policy: &[CapabilityGrant]) -> Option<Connection> {
    let allowed: BTreeSet<_> = policy
        .iter()
        .filter(|grant| {
            grant.provider == connection.provider
                && grant.account_id == connection.account_id
                && grant.channel_id == connection.channel_id
        })
        .flat_map(|grant| grant.capabilities.iter().copied())
        .collect();
    let mut result = connection.clone();
    result.capabilities = connection
        .capabilities
        .intersection(&allowed)
        .copied()
        .collect();
    (!result.capabilities.is_empty()).then_some(result)
}
fn valid_action(action: &ClientAction) -> bool {
    match action {
        ClientAction::SendChat { text } => !text.trim().is_empty() && text.len() <= 8192,
        ClientAction::DeleteChat { message_id } => valid_id(message_id, 256),
        ClientAction::TimeoutUser {
            user_id,
            duration_seconds,
        } => identity(user_id) && (1..=1_209_600).contains(duration_seconds),
    }
}
fn validate_registration(request: &ClientRegistration) -> Result<(), String> {
    if request.version != CLIENT_EXECUTION_VERSION || request.generation == 0 {
        return Err("invalid_argument: registration version or generation".into());
    }
    if request.connections.is_empty() || request.connections.len() > MAX_CONNECTIONS {
        return Err("invalid_argument: registration connection count".into());
    }
    let mut ids = BTreeSet::new();
    for connection in &request.connections {
        if !identity(&connection.id) || !ids.insert(&connection.id) {
            return Err("invalid_argument: duplicate or invalid connection identity".into());
        }
        if !identity(&connection.provider)
            || !identity(&connection.account_id)
            || !identity(&connection.channel_id)
        {
            return Err("invalid_argument: provider account/channel identity".into());
        }
    }
    Ok(())
}
fn intent_time(id: &str, now: u64) -> Result<u64, String> {
    let created = id
        .split_once(':')
        .and_then(|(time, suffix)| {
            valid_id(suffix, 128)
                .then(|| time.parse::<u64>().ok())
                .flatten()
        })
        .ok_or("invalid_argument: intent must be timestamp:unique_id")?;
    if id.len() > 160
        || created > now.saturating_add(10000)
        || created.saturating_add(INTENT_RETENTION_MS) <= now
    {
        return Err("conflict: expired or invalid intent".into());
    }
    Ok(created)
}
impl ExecutionRegistry {
    pub fn set_policy(
        &mut self,
        workspace: &str,
        policy: Vec<CapabilityGrant>,
        now: u64,
    ) -> Result<(), String> {
        if !identity(workspace)
            || policy.len() > 64
            || policy.iter().any(|p| {
                !identity(&p.provider) || !identity(&p.account_id) || !identity(&p.channel_id)
            })
        {
            return Err("invalid_argument: capability policy".into());
        }
        if !self.policies.contains_key(workspace) && self.policies.len() >= MAX_WORKSPACES {
            return Err("unavailable: workspace policy limit".into());
        }
        for ((owner, _), registration) in &mut self.clients {
            if owner == workspace {
                registration.status.connections = registration
                    .requested
                    .connections
                    .iter()
                    .filter_map(|c| granted(c, &policy))
                    .collect();
                registration.active &= !registration.status.connections.is_empty();
            }
        }
        self.policies.insert(workspace.into(), policy);
        self.maintain(now);
        Ok(())
    }

    pub fn register(
        &mut self,
        workspace: &str,
        actor: &str,
        request: ClientRegistration,
        now: u64,
    ) -> Result<ExecutorStatus, String> {
        self.maintain(now);
        if !identity(workspace) || !identity(actor) {
            return Err("invalid_argument: workspace or client identity".into());
        }
        validate_registration(&request)?;
        let key = (workspace.to_owned(), actor.to_owned());
        if let Some(old) = self.clients.get_mut(&key) {
            if request.generation < old.requested.generation
                || (request.generation == old.requested.generation
                    && (!old.active || old.requested != request))
            {
                return Err("conflict: stale or changed registration".into());
            }
            if request.generation == old.requested.generation {
                old.status.expires_at_ms = now.saturating_add(REGISTRATION_TTL_MS);
                return Ok(old.status.clone());
            }
        } else if self.clients.len() >= MAX_CLIENTS {
            return Err("unavailable: client registration limit".into());
        }
        let policy = self
            .policies
            .get(workspace)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let connections: Vec<_> = request
            .connections
            .iter()
            .filter_map(|c| granted(c, policy))
            .collect();
        if connections.is_empty() {
            return Err("forbidden: no authorized client capabilities".into());
        }
        let mut channels = BTreeSet::new();
        if connections
            .iter()
            .any(|c| !channels.insert((&c.provider, &c.channel_id)))
            || self.clients.iter().any(|((owner, other), r)| {
                owner == workspace
                    && other != actor
                    && r.active
                    && r.status.connections.iter().any(|old| {
                        connections
                            .iter()
                            .any(|c| c.provider == old.provider && c.channel_id == old.channel_id)
                    })
            })
        {
            return Err("conflict: channel already has an active executor".into());
        }
        let status = ExecutorStatus {
            client_id: actor.into(),
            generation: request.generation,
            expires_at_ms: now.saturating_add(REGISTRATION_TTL_MS),
            connections,
        };
        self.clients.insert(
            key,
            Registration {
                requested: request,
                status: status.clone(),
                active: true,
            },
        );
        self.maintain(now);
        Ok(status)
    }

    fn current(
        &self,
        workspace: &str,
        actor: &str,
        generation: u64,
    ) -> Result<&ExecutorStatus, String> {
        self.clients
            .get(&(workspace.into(), actor.into()))
            .filter(|r| r.active && r.status.generation == generation)
            .map(|r| &r.status)
            .ok_or_else(|| "unavailable: client registration is not active".into())
    }
    pub fn connection(
        &mut self,
        workspace: &str,
        actor: &str,
        generation: u64,
        id: &str,
        now: u64,
    ) -> Result<Connection, String> {
        self.maintain(now);
        self.current(workspace, actor, generation)?
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| "forbidden: connection not registered".into())
    }
    pub fn unregister(
        &mut self,
        workspace: &str,
        actor: &str,
        generation: u64,
        now: u64,
    ) -> Result<(), String> {
        self.maintain(now);
        self.current(workspace, actor, generation)?;
        self.clients
            .get_mut(&(workspace.into(), actor.into()))
            .unwrap()
            .active = false;
        self.maintain(now);
        Ok(())
    }
    pub fn connections(&mut self, workspace: &str, now: u64) -> Vec<ExecutorStatus> {
        self.maintain(now);
        self.clients
            .iter()
            .filter(|((owner, _), r)| owner == workspace && r.active)
            .map(|(_, r)| r.status.clone())
            .collect()
    }
    pub fn withdraw_workspace(&mut self, workspace: &str, now: u64) {
        for ((owner, _), registration) in &mut self.clients {
            if owner == workspace {
                registration.active = false;
            }
        }
        self.maintain(now);
    }

    pub fn enqueue(
        &mut self,
        workspace: &str,
        request: InstructionRequest,
        now: u64,
    ) -> Result<Instruction, String> {
        self.maintain(now);
        let InstructionRequest {
            id,
            request_id,
            client_id: actor,
            generation,
            connection_id: connection,
            action,
            expires_at_ms: expires,
        } = request;
        let created = intent_time(&id, now)?;
        let key = (workspace.to_owned(), id.to_owned());
        if let Some(old) = self.instructions.get(&key) {
            let prior = &old.instruction;
            if prior.request_id != request_id
                || prior.client_id != actor
                || prior.generation != generation
                || prior.connection_id != connection
                || prior.action != action
                || prior.expires_at_ms != expires
            {
                return Err("conflict: intent reused with different arguments".into());
            }
            return Ok(prior.clone());
        }
        if request_id.as_ref().is_some_and(|id| !valid_id(id, 160))
            || !valid_action(&action)
            || expires <= now
            || expires > now.saturating_add(MAX_INSTRUCTION_LIFETIME_MS)
            || expires > created.saturating_add(INTENT_RETENTION_MS)
        {
            return Err("invalid_argument: client instruction".into());
        }
        let status = self.current(workspace, &actor, generation)?;
        let binding = status
            .connections
            .iter()
            .find(|c| c.id == connection && c.capabilities.contains(&action.capability()))
            .cloned()
            .ok_or("forbidden: action capability is unavailable")?;
        if self.instructions.len() >= MAX_INSTRUCTIONS {
            return Err("unavailable: instruction retention limit".into());
        }
        let order = self
            .next_order
            .checked_add(1)
            .ok_or("unavailable: instruction sequence limit")?;
        let instruction = Instruction {
            connection: binding,
            id,
            request_id,
            workspace_id: workspace.into(),
            client_id: actor,
            generation,
            connection_id: connection,
            action,
            created_at_ms: created,
            expires_at_ms: expires,
            state: InstructionState::Queued,
            external_id: None,
        };
        self.instructions.insert(
            key,
            Pending {
                instruction: instruction.clone(),
                order,
                lease: None,
                receipt: None,
            },
        );
        self.next_order = order;
        Ok(instruction)
    }

    pub fn claim(
        &mut self,
        workspace: &str,
        actor: &str,
        generation: u64,
        limit: usize,
        now: u64,
    ) -> Result<Vec<ClaimedInstruction>, String> {
        self.maintain(now);
        self.current(workspace, actor, generation)?;
        if !(1..=64).contains(&limit) {
            return Err("invalid_argument: claim limit must be 1..64".into());
        }
        let mut next = self.next_lease;
        next.checked_add(limit as u64)
            .ok_or("unavailable: execution lease limit")?;
        let mut result = vec![];
        let mut candidates: Vec<_> = self
            .instructions
            .iter()
            .filter(|(_, p)| {
                p.instruction.workspace_id == workspace
                    && p.instruction.client_id == actor
                    && p.instruction.generation == generation
                    && p.instruction.state == InstructionState::Queued
            })
            .map(|(key, p)| (key.clone(), p.order))
            .collect();
        candidates.sort_unstable_by_key(|(_, order)| *order);
        for (key, _) in candidates.into_iter().take(limit) {
            let pending = self.instructions.get_mut(&key).unwrap();
            next += 1;
            pending.lease = Some(next);
            pending.instruction.state = InstructionState::InFlight;
            result.push(ClaimedInstruction {
                instruction: pending.instruction.clone(),
                lease: next,
            });
        }
        self.next_lease = next;
        Ok(result)
    }

    pub fn report(
        &mut self,
        workspace: &str,
        actor: &str,
        generation: u64,
        receipt: ExecutionReceipt,
        now: u64,
    ) -> Result<(), String> {
        self.maintain(now);
        self.current(workspace, actor, generation)?;
        if receipt
            .external_id
            .as_ref()
            .is_some_and(|id| !valid_id(id, 256))
        {
            return Err("invalid_argument: provider receipt identity".into());
        }
        let pending = self
            .instructions
            .get_mut(&(workspace.into(), receipt.instruction_id.clone()))
            .ok_or("not_found: client instruction")?;
        if pending.instruction.client_id != actor
            || pending.instruction.generation != generation
            || pending.lease != Some(receipt.lease)
        {
            return Err("forbidden: execution lease does not belong to this client".into());
        }
        if let Some(prior) = &pending.receipt {
            return if *prior == receipt {
                Ok(())
            } else {
                Err("conflict: receipt is already final".into())
            };
        }
        if pending.instruction.state != InstructionState::InFlight {
            return Err("conflict: instruction no longer accepts results".into());
        }
        pending.instruction.state = receipt.outcome.into();
        pending.instruction.external_id = receipt.external_id.clone();
        pending.receipt = Some(receipt);
        Ok(())
    }

    pub fn instructions(&mut self, workspace: &str, now: u64) -> Vec<Instruction> {
        self.maintain(now);
        self.instructions
            .iter()
            .filter(|((owner, _), _)| owner == workspace)
            .map(|(_, p)| p.instruction.clone())
            .collect()
    }
    pub fn instruction(&mut self, workspace: &str, id: &str, now: u64) -> Option<Instruction> {
        self.maintain(now);
        self.instructions
            .get(&(workspace.into(), id.into()))
            .map(|p| p.instruction.clone())
    }

    fn maintain(&mut self, now: u64) {
        for registration in self.clients.values_mut() {
            if registration.status.expires_at_ms <= now {
                registration.active = false;
            }
        }
        for pending in self.instructions.values_mut() {
            let item = &mut pending.instruction;
            if !matches!(
                item.state,
                InstructionState::Queued | InstructionState::InFlight
            ) {
                continue;
            }
            let active = self
                .clients
                .get(&(item.workspace_id.clone(), item.client_id.clone()))
                .is_some_and(|r| {
                    r.active
                        && r.status.generation == item.generation
                        && r.status.connections.iter().any(|c| {
                            c.id == item.connection_id
                                && c.capabilities.contains(&item.action.capability())
                        })
                });
            if !active || item.expires_at_ms <= now {
                item.state = if item.state == InstructionState::InFlight {
                    InstructionState::Unknown
                } else if item.expires_at_ms <= now {
                    InstructionState::Expired
                } else {
                    InstructionState::Cancelled
                };
            }
        }
        self.instructions.retain(|_, p| {
            p.instruction
                .created_at_ms
                .saturating_add(INTENT_RETENTION_MS)
                > now
        });
    }
}
