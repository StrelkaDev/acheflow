# Spec: client execution and workspace capabilities

Status: accepted architecture; implementation pending, 2026-11-03.
Governed by [ADR-002](decisions/002-client-execution-and-workspace-capabilities.md).
This describes a target contract, not existing endpoints or registered MCP tools.

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
6. Remove the legacy server credential/input/transport paths for integrations after
   the migrated paths pass. Keep live-account checks distinct from provider-shaped
   fixtures; do not claim migration complete from mocks or protocol-only tests.

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
