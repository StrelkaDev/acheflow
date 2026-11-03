# Client execution: setup and current boundary

The logic server owns workspace state, policy and rules. Provider accounts, bot
accounts, OAuth and external AI keys belong to the client or an optional separately
operated executor. Every installation can use its own platform applications/accounts.
Public OAuth application IDs may be shared configuration; private application secrets
and account tokens must never be sent to the logic server.

## Implemented pieces

- `acheflow-client::runtime::ProviderRuntime`: native real WASM reactor with local
  credentials, bounded HTTPS/WSS, opaque handles and a local instruction ledger.
- `acheflow-client::bridge::ExecutorConnection`: authenticated MCP registration,
  typed event publication, instruction claims, results and unregister.
- Server `acheflow_executor`: `register`, `publish`, `claim`, `report`, `unregister`.
- Common `send_chat`: queues one instruction per client-owned channel; receipts
  and provider echoes update the normal shared chat history.

This is a library for integrating a native UI or executor application. There is
currently no completed standalone executor binary or main-UI account connection
flow. Enabling a provider on the server alone does not collect or send anything.
Browser runtime, OAuth completion/renewal for every platform, durable credential
storage, native polls and generic external MCP tool registration remain pending.
Moderation DTOs exist; unsupported provider handlers cannot be registered.

## Authenticate the executor

The server still needs its own Acheflow transport credentials to distinguish
operators, readers and executors. These are independent of platform/AI credentials.
MCP binds to loopback by default, validates Origin, limits request bodies to 1 MiB
and uses the pinned MCP SDK. Remote use requires an explicitly configured secure
proxy/tunnel; the native adapter permits HTTPS or loopback HTTP and refuses redirects.

Set `ACHEFLOW_MCP_TOKEN` to a generated operator credential (32+ characters).
Optionally set a distinct `ACHEFLOW_MCP_READ_TOKEN`. Configure executor credentials
using `ACHEFLOW_MCP_EXECUTORS`, a JSON array with distinct generated tokens:

```json
[
  {
    "id": "desktop-main",
    "workspace_id": "YOUR_WORKSPACE_ID",
    "token": "YOUR_DISTINCT_GENERATED_ACHEFLOW_TRANSPORT_TOKEN"
  }
]
```

The executor connects to `http://127.0.0.1:21296/mcp/executor/desktop-main` with its
own Bearer credential. The endpoint pins actor and workspace on the server; those
fields cannot be supplied in executor operation bodies. An executor cannot call
operator/read tools or use another executor's token. No platform tokens appear in
this configuration. Treat executors granted access to a workspace as trusted event
sources for that workspace; provider account identity is asserted by the executor.

## Save policy, then register availability

Enable the provider in this workspace. Use operator `acheflow_command` with the
current workspace/revision, a `unix_milliseconds:unique_suffix` intent ID and:

```json
{
  "type": "set_client_capabilities",
  "grants": [
    {
      "provider": "EXTENSION_ID",
      "account_id": "AUTHORIZED_PLATFORM_ACCOUNT_ID",
      "channel_id": "CHANNEL_ID",
      "capabilities": ["chat.read", "chat.send", "audience.read"]
    }
  ]
}
```

Only grant implemented capabilities. Policy persists as workspace
`client_capabilities`; default is empty/deny. The client sends connection metadata
through `register` after local authorization, using a monotonic generation and
connection IDs local to that client. Permissions are the intersection of policy,
implemented provider handlers and requested live capabilities. Automatic registration
does not rewrite saved policy. Only one executor may own a provider/channel at once.
Switching an account, workspace or expired session requires a new generation.

Repeat the identical registration before its 30-second TTL expires. Client callers
own scheduling, local panels/ticks, socket reads and shutdown. `claim` returns typed
instructions once; the local adapter validates workspace, client, generation,
connection, capabilities and expiry before executing. Report `sent` only on a
confirmed provider response; interrupted/uncertain writes are `unknown`. Never
retry them automatically. `disconnect` unregisters and stops the MCP session; dispose
the provider runtime to cancel its remaining sockets and local state.

Only normalized chat/audience/deletion events are forwarded. Raw provider JSON,
authorization panels, OAuth responses, local errors, storage and credential handles
remain client-local. Unknown event fields and arbitrary effects are rejected.
Disabled providers and withdrawn policy immediately lose server authority. Queued
work is cancelled/expired; in-flight uncertainty is retained without reassignment.

## Persistence and verification limits

The active server workspace and public capability policy are in memory too; saving
policy through MCP does not write a server workspace database. A connected client
reconciles policy into its local catalog/exports. After server restart, apply that
saved workspace and register the executor again. See [settings persistence and
restore](settings-persistence.md) for exact storage and conflict behavior.

Registrations, instruction receipts and local captured credentials are in memory.
Do not replay external sends after a server/client process restart. Durable intent
journals and vault-backed account sessions must be implemented before unattended
crash recovery. Existing exports with removed manual token fields must have those
fields removed before being submitted to the server; old secret-write requests fail.

Tests use two independently instantiated real WASM fixture providers over real MCP
HTTP, including fanout, receipts, echo merging, cross-actor rejection and disconnect.
Actual Twitch/YouTube/VK WASM startup/normalization tests are separate. They do not
prove live account authorization, moderator permissions or browser behavior. Platform
applications must still be registered by the installation administrator; credentials
must not be embedded in open-source artifacts or sent in chats.
