# Spec: client execution and workspace capabilities

Status: server migration and native client/MCP libraries implemented, 2026-11-03.
Governed by [ADR-002](decisions/002-client-execution-and-workspace-capabilities.md).
Implemented boundaries and remaining work are separated below.

## Current implementation boundary

The first slice adds typed credential-free registrations, grants, actions,
instructions and receipts in `acheflow-extension-api::client_execution`, plus the
pure `acheflow-control::client_execution::ExecutionRegistry`. Registrations are
intersected with manually supplied workspace policy. The outbox issues exclusive
leases in enqueue order, binds receipts to actor/generation, preserves final
results on retries and rejects changed intent payloads. Expired/disconnected
in-flight work becomes `unknown`, never an automatic retry.

Limits in this slice: 64 retained client identities, 16 connections per registration,
64 workspace policies with up to 64 grants each, 512 retained instructions, claims
of 1..64 instructions, a 30-second registration heartbeat lifetime, a maximum
five-minute instruction lifetime, and ten-minute intent retention. Registration
tombstones remain bounded until process restart, so an expired generation cannot
be revived by a delayed request. Intent IDs are `timestamp:unique_id`; after
retention, old IDs are rejected rather than treated as new work. State is in memory;
durable client execution deduplication across process crashes is not implemented.

Workspace JSON persists `client_capabilities` policy separately from extension public
configuration. Durability is provided by the client catalog/workspace export, not a
server workspace database: the active server copy is in memory and must be applied
again after restart. [Persistence and conflict behavior](settings-persistence.md)
records the verified boundary. `set_client_capabilities` updates it through the common revision-checked
control API. Snapshots expose policy and only the active workspace's live executors.
Switching workspace, disabling a provider, policy revocation, disconnect and heartbeat
expiry withdraw live authority. One executor owns each provider/channel at a time.

`/mcp/executor/<id>` authenticates a distinct Acheflow transport token pinned to an
actor/workspace. Its only tool, `acheflow_executor`, implements `register`, `unregister`,
`publish`, `claim` and `report`; actor/workspace cannot be changed in request bodies.
Supported public events are chat, audience and deletion/clear-chat. Arbitrary JSON
effects, OAuth responses, panels and tokens are rejected. `send_chat` queues one typed
instruction per client-owned destination; exclusive claims and receipts update the
normal merged-chat delivery/echo state. External receipt errors are not forwarded.
Operator/read credentials cannot be reused as executor credentials.

Manifests declare `execution_side: server|client`. Server is the safe default and
rejects origins, HTTP/WebSocket capabilities and credential declarations. Client
installations retain public discovery metadata on the server; their modules are never
loaded there. There is no `legacy_server` mode, server credential store, secret-write
control action or server provider transport.

`acheflow-client` supplies a native WASM runtime, local captured credentials, bounded
HTTPS/WSS, cancellation, identity/expiry/capability checks, a local intent ledger and
a typed MCP adapter. Real loopback tests connect two independent WASM fixture providers
and verify fanout, receipts, echo reconciliation and replay protection. Actual provider
guests are separately exercised without accounts. This is a library integration, not
a finished account-connection UI or always-on executor application.

Still pending: native/browser `Connect account` integration, application-registration
setup, platform-specific OAuth completion/renewal, durable local credential/intent
storage, browser transport, arbitrary MCP tool descriptors and native poll instructions.
The bridge currently accepts implemented chat/audience capabilities; moderation policy
and action DTOs exist but provider handlers are not advertised until implemented.
There is no fallback to server-held keys. For setup see [client-executor.md](client-executor.md).

## Objective

Move integration credentials and provider execution to clients. Keep workspace
state and decision logic on the Acheflow server. Support manual MCP configuration
and automatic capability registration by the connected client for its active
workspace, without forwarding provider or AI secrets.

## Shared contract

| Operation | Required domain data |
|---|---|
| Register capabilities | Workspace, authenticated client session, registration generation, extension/contract versions, public connection/account/channel identities, available typed capabilities |
| Publish events | Workspace, registration generation, source connection, event IDs/cursor, event/observation timestamps, bounded normalized payloads |
| Retrieve instructions | Workspace, registration generation, bounded cursor/limit, claimed execution lease where applicable |
| Report results | Stable intent ID, registration generation, lease, sent/rejected/failed/unknown outcome, optional provider receipt ID, bounded redacted error code |
| Withdraw registration | Workspace, client session and registration generation |

The server derives the actor from authenticated transport identity rather than
trusting a supplied client ID. Capability availability is intersected with saved
workspace permissions. Personal/bot sender identities remain distinct and explicit.
An executor claims work for one connection/account; execution is not broadcast to
every connected client. Stale generations, wrong workspaces and expired intents
cannot acquire a lease or update a completed result.

Public metadata is allowlisted and schema-validated; no raw OAuth response,
authorization header, token-bearing URL, arbitrary executable code or unrestricted
provider request crosses the bridge. Chat content and AI results remain untrusted.
The server returns domain actions and the client maps them to provider operations.

Manual configuration is saved intent/policy. Client registration is live session
state and never silently replaces that policy. Client disconnect, workspace switch,
logout and extension disable remove live availability and cancel owned work. Report
gaps, waiting-for-client state and uncertain external outcomes visibly.

## MCP adapter

Candidate application tool names are `acheflow_register_client`,
`acheflow_publish_events`, `acheflow_next_actions`, `acheflow_report_results` and
`acheflow_unregister_client`. Schemas, bounds and the supported revision must be
fixed during implementation and checked against the installed SDK.

On MCP 2026-07-28 the client initiates these calls; tool results carry instructions
for the client's executor. This is not a server-initiated `tools/call` to the client.
The existing server state/command tools remain adapters over shared application
handlers. UI and MCP must not acquire separate stores or authorization semantics.

## Implementation slices

1. Add typed registration/event/instruction/result contracts and a pure domain
   registry/queue. Verify workspace isolation, policy intersection, leases,
   expiry, replay, duplicate ingestion and uncertain writes.
2. Add paired extension manifests/runtimes with explicit client/server capabilities.
   Move provider I/O to trusted client parts; keep server parts credential-free.
3. Add client-local Connect account, renewal and credential lifecycle, independently
   verified for native/browser targets and each provider. Document application
   registration and bot/moderator permissions; do not embed production secrets.
4. Connect client ingestion/execution to shared server handlers and the MCP adapter.
   Check two real WASM providers through the full events -> instruction -> provider
   result path, including receipt/echo reconciliation.
5. Add the workspace UI for manual MCP configuration, live client capabilities,
   sender selection, unavailable tools, reconnects and per-action errors. Verify the
   actual native/browser interactions and a final visual check.
6. The owner requested removal of the legacy server path now; this is complete.
   Complete the remaining client flows without reintroducing server credentials.
   Keep live-account checks distinct from provider-shaped fixtures.

## Acceptance

- No platform/bot/OAuth/private application/AI credential reaches the logic server,
  including on reconnect, failed token exchange or provider error handling.
- Manual and automatic configuration converge on one workspace registry while
  keeping saved policy distinct from live client availability.
- Independent clients/servers/workspaces do not change sender identities or share
  registrations implicitly. Multiple executors do not duplicate external effects.
- Revocation, cancellation, stop/restart, disconnect, stale events/results and unknown
  delivery preserve visible state without hidden retries.
- With no executor, external work is unavailable; an optional headless executor can
  restore service without placing credentials in the logic-server process.
- Direct client sending, collection and moderation are checked against actual
  provider permissions. Remote/managed AI use is explicit and optional.
