# Provider execution: server and client hosts

The server owns workspace state, policy and rules. Provider WASM can execute in an
explicitly configured trusted server reactor or in a device-owned native runtime.
Both hosts use the same bounded transport and typed event/instruction contract.
Accounts remain separate: configuring server execution never imports a device's
local credentials. Explicit [account copying](extension-authorization.md#connect-a-trusted-server-account)
is available. Execution through personal accounts and per-user delegation remain
[SHARED-06](roadmap.md#shared-06-authorize-server-accounts-and-migrate-credentials).

## Implemented pieces

- `acheflow-client::runtime::ProviderRuntime`: native real WASM reactor with host-private
  credentials, bounded HTTPS/WSS, opaque handles and a local instruction ledger.
- `acheflow-client::bridge::ExecutorConnection`: authenticated MCP registration,
  typed event publication, instruction claims, results and unregister.
- Server `acheflow_executor`: `register`, `publish`, `claim`, `report`, `unregister`.
- Trusted server executor: one owned reactor per enabled provider binding, with
  in-process registration/publication/claims and lifecycle fencing.
- Common `send_chat`: queues one instruction per authorized channel; receipts
  and provider echoes update the normal shared chat history.

With **Studio chat** selected, the desktop automatically starts a native runtime
for an enabled, locally authorized provider in the active server workspace. It
schedules provider ticks, socket/stream receives, normalized event publication
and instruction claims independently of the visible page. **My messages** keeps
personal authorization separate and does not start collection. Account removal,
disable and workspace/server changes cancel the runtime and unregister its
channels; failed cleanup expires under the registration TTL. There is no
standalone executor binary. Local collection is suppressed when the provider is
assigned to the server.

Studio uses `POST /api/control/executor` with an authenticated device session and
the **Run extensions** grant, the same API body limit and configured UI origins.
The server namespaces executor identity by the authenticated device; a caller
cannot impersonate another device by choosing its `client_id`. Device revocation
withdraws its registrations. See [device access](server-access.md).
Its typed envelope contains `workspace_id`, a `studio-` client identity and an
`ExecutorRequest`; it accepts no provider credentials or raw platform effects.
External executors still use the separately authenticated MCP endpoint described
below. Both adapters use the same workspace, generation, policy and source checks.
The first discovered channel of an enabled local integration creates its workspace
capability grant through a separate operator command. Existing narrower grants are
preserved; registration itself never changes policy. A live revocation is not
silently reversed by a connection retry. The server workspace must be applied
before its live chat can connect. See [extension authorization](extension-authorization.md).
Continuous browser execution, live provider-operation qualification, durable intent
storage, native poll instructions and generic external MCP tool registration remain
unfinished. Native macOS local account panels already use the system vault.
Moderation DTOs exist; the built-in provider runtime rejects unsupported moderation
actions. Separately implemented executors must advertise only handlers they support.

## Trusted server execution

Server provider packages are discovered from the server's extension directory
(`ACHEFLOW_EXTENSIONS_DIR` or its normal installation path), separately from pure
logic guests. Their existing `execution_side: "client"` identifies the bounded
provider/reactor contract; it no longer implies that only a desktop may host them.
Pure `server` logic guests still cannot request network I/O or credentials.

An applied workspace may contain this public configuration fragment. Replace the
placeholders with an installed provider ID, its public settings and the verified
account/channel selected for the server:

```json
{
  "server_integrations": {
    "provider-id": {
      "enabled": true,
      "application": "SERVER_ACCOUNT_SLOT",
      "account_id": "ACCOUNT_ID",
      "channel_id": "CHANNEL_ID",
      "config": {}
    }
  },
  "client_capabilities": [{
    "provider": "provider-id",
    "account_id": "ACCOUNT_ID",
    "channel_id": "CHANNEL_ID",
    "capabilities": ["chat.read", "chat.send"]
  }]
}
```

This is a fragment of a complete workspace, not a standalone import. Apply requires
the normal management grants and expected revision. Settings must match nonsecret
manifest declarations. One primary binding per provider is supported. A collector
requires `chat.read`; sending additionally requires `chat.send`. The discovered
channel must match the binding; channel discovery never expands workspace grants.
Personal moderation/action-only sessions are separate SHARED-06 work.

The server uses a dedicated server-ID/workspace vault namespace with isolated
provider/application/account slots. It never falls back to a local device account.
A missing/locked account or provider authorization loss yields `needs_authorization`
and withdraws live authority; configuring a binding cannot manufacture credentials.
Use the explicit [trusted-server account connection](extension-authorization.md#connect-a-trusted-server-account)
to revalidate and copy an authorized account. `application` is the server-issued
opaque slot handle; public OAuth application settings remain in `config`.
Personal user-to-account ownership is implemented through the separate
[personal account flow](extension-authorization.md#personal-accounts-on-the-trusted-server).
Per-action authority and delegation remain unfinished. macOS uses the existing OS
vault adapter; other platforms still need their server credential adapter.

Provider URLs, OAuth validation/renewal, subscriptions and protocol mapping remain
in WASM. Rotated credentials checkpoint immediately before the guest can issue another
request; opaque storage-only updates are coalesced to avoid per-message vault writes. Failed checkpointing stops the executor for reauthorization
rather than restarting with stale stored credentials. Panels, raw HTTP payloads
and credential values are not forwarded through the typed event bridge.

An enabled binding reserves its provider/channel against external collector
registration, including while awaiting authorization. The native UI waits for server
ownership metadata before starting a local collector. Reconciliation preserves an
unchanged reactor; replacement retires its old identity before starting another.
Disable, policy revocation, workspace replacement and shutdown cancel owned I/O,
withdraw channels and fence late events. Server restart restores configuration
without activating integrations; explicit Apply starts them again.

Lease maintenance continues while provider turns wait for HTTP/gRPC or sends.
Transient transport errors reconnect with bounded backoff. Unknown/in-flight sends
become final uncertain receipts when authority is withdrawn; another generation
cannot claim or retry them. Permanently fenced server actors release registry
capacity without erasing receipt history or weakening normal client lease rules.
The outbox remains in memory; durable intent storage is still unfinished.

**Extensions → Server** shows configured server integrations and their connection/
authorization state. `ControlSnapshot.server_providers` exposes only bounded public
metadata; account identifiers require extension-configuration permission. Package
installation and account connection remain separately authorized operations.

Lifecycle tests use two real WASM fixture installations for collection, sending,
replacement, revocation, repeated replacement and uncertain receipts. Independent
YouTube/VK WASM protocol tests cover synthetic receive/send paths and bounded
responses. These checks do not qualify live platform accounts; live authorization
and renewal remain part of shared-workflow qualification.

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

Client provider installations and public config belong to the client. The page
separates **Client integrations**, **Server logic** and **Workspace capabilities**;
the server no longer publishes a provider installation catalog. Native discovery
and browser bundling are described in [extensions.md](extensions.md). There is no
requirement to install a provider on the server before registering it.

Use operator `acheflow_command` with the
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
connection IDs local to that client. Server permissions are the intersection of saved policy and requested live
capabilities; the trusted executor must advertise only its implemented handlers.
Host identities and IDs owned by server logic cannot be claimed as client providers. Automatic registration
does not rewrite saved policy. Only one executor may own a provider/channel at once.
Switching an account, workspace or expired session requires a new generation.

Repeat the identical registration before its 30-second TTL expires. Studio owns scheduling, local panels/ticks, socket reads and shutdown for native
integrations; separately implemented executors must own the same lifecycle. `claim` returns typed
instructions once; the local adapter validates workspace, client, generation,
connection, capabilities and expiry before executing. Report `sent` only on a
confirmed provider response; interrupted/uncertain writes are `unknown`. Never
retry them automatically. `disconnect` unregisters and stops the MCP session; dispose
the provider runtime to cancel its remaining sockets and local state.

Only normalized chat/audience/deletion events are forwarded. Raw provider JSON,
authorization panels, OAuth responses, local errors, storage and credential handles
remain client-local. Unknown event fields and arbitrary effects are rejected.
Withdrawing policy, unregistering, or heartbeat expiry removes server authority.
A local enable/disable preference alone is not a remote revocation mechanism; the
owning client must stop its runtime and unregister when disabled. Queued
work is cancelled/expired; in-flight uncertainty is retained without reassignment.

## Registry bounds and intent semantics

| Resource | Current limit |
| --- | --- |
| Retained client identities | 64 |
| Connections per registration | 16 |
| Retained workspace policies / grants per policy | 64 / 64 |
| Retained instructions | 512 |
| Instructions per claim | 1–64 |
| Registration heartbeat lifetime | 30 seconds |
| Maximum instruction lifetime | Five minutes |
| Intent retention | Ten minutes |

Generation tombstones stay bounded until process restart so delayed requests cannot
revive old authority. Intent IDs use `timestamp:unique_id`; an intent outside retention
is rejected instead of silently becoming new work. Exclusive claims are bound to
actor, generation and connection. Changed payloads cannot reuse an existing intent;
expired/disconnected in-flight work becomes unknown without automatic reassignment.
These limits belong to the execution registry, separate from guest fuel/queue bounds.

## Persistence and verification limits

The active server workspace and public capability policy are in memory too; saving
policy through MCP does not write a server workspace database. A connected client
reconciles policy into its local catalog/exports. After server restart, apply that
saved workspace and register the executor again. See [settings persistence and
restore](settings-persistence.md) for exact storage and conflict behavior.

Registrations, instruction receipts and local captured credentials are in memory.
Do not replay external sends after a server/client process restart. Durable intent
journals and explicit reconciliation with restored local account sessions are
required before unattended crash recovery; a saved account is not an execution
journal. Existing exports with removed manual token fields must have those fields
removed before being submitted to the server; old secret-write requests fail.

Tests use two independently instantiated real WASM fixture providers over real MCP
HTTP, including fanout, receipts, echo merging, cross-actor rejection and disconnect.
Actual Twitch/YouTube/VK WASM startup/normalization tests are separate. They do not
prove live account authorization, moderator permissions or browser behavior. Platform
applications must still be registered by the installation administrator; credentials
must not be embedded in open-source artifacts or sent in chats.
