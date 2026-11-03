# ADR-002: Client-owned execution and workspace capability registration

## Status

Accepted by the project owner, 2026-11-03. Implementation pending.
Supersedes [ADR-001](001-provider-authentication.md), including its server-held
bot credentials and server-side provider execution.

## Context

The owner requires platform authorization and execution to stay on clients.
The Acheflow logic server receives necessary events and public metadata, evaluates
rules, and returns instructions. It must hold no platform, bot or external AI
credentials. This applies to bot accounts as well as personal accounts.

Workspace capabilities can be configured manually or registered automatically by
a connected client for its active workspace. Neither path transfers credentials
or lets client data grant itself authority. The current server extension runtime,
secret store and direct provider sends do not yet implement this model.

## Decision

### Client executor and logic server

| Responsibility | Owner |
|---|---|
| Platform application configuration, OAuth, token renewal and local credential storage | Client or separately operated executor agent |
| Platform connections, chat collection and provider protocol parsing | Client-side extension |
| Provider-neutral state, rules, scheduling and action decisions | Acheflow logic server and server-side extension |
| Public channel, account and available-capability metadata | Client reports to its authorized workspace |
| Provider API calls, including bot replies and moderation | Authorized client executor |
| Delivery receipts and observed provider echoes | Client reports results; server reconciles shared state |
| Private AI provider keys and credential-bearing AI calls | Client/executor side |

The server may run rules or local computation without external service keys. If a
decision needs an external AI service, a client-side tool performs that call with
its own credentials and returns the necessary result. AI output and chat text are
untrusted data; neither bypasses action validation.

The credential boundary concerns external integration/AI secrets. It does not
remove authentication or HTTPS from client-server communication. No access/refresh
token, private client secret, authorization header, token-bearing URL or AI API key
is included in registration, events, instructions, results, logs or workspace
exports. Public OAuth client IDs may be configuration but are not account tokens.

### Paired extensions

One extension package can have client and server parts with a shared, versioned
domain contract. The client part owns platform I/O and credentials; the server part
owns provider-neutral policy and instruction creation. A server part cannot invoke
provider HTTP/WebSocket operations or request/export credentials. Client code that
handles credentials must come from a trusted installation; a remote logic server
cannot inject arbitrary executable adapters or credential-bearing destinations.

An instruction describes a typed action such as sending a message, deleting a
message or timing out a user. It contains no executable script, raw provider HTTP
request or arbitrary URL. The client checks its local policy, sender identity,
workspace, channel, granted capability and freshness before executing. Automatic
execution is allowed within explicitly enabled local automation policy; the server
cannot expand that policy by advertising a new capability.

### Workspace registry: manual configuration and client registration

Both configuration paths produce entries in a common workspace capability registry:

- Manual configuration saves the desired MCP integrations/tools, public endpoint
  metadata, routes and workspace policy. Reachability and successful authorization
  must still be established before a capability becomes available.
- A connected client registers its installed extension/tool contracts, public
  account/channel identities and currently available capabilities for its active
  workspace. Its registration is bound to an authenticated client session and
  generation; registration claims do not grant new workspace permissions.

Saved desired configuration and live availability are separate. Automatic
registration does not overwrite manual policy, silently persist imported secrets,
or enable an unapproved capability. Conflicts and incompatible contract versions
are visible. A workspace switch or disconnect withdraws the previous live
registration; stale registrations and results are rejected. One workspace cannot
reuse another workspace's client connection or targets.

Where a manually configured MCP endpoint requires a private external credential,
an authorized client/agent establishes that connection and advertises its tools.
The logic server does not receive the credential as a shortcut. Manual endpoint
configuration alone does not make a local client endpoint reachable from a server
behind a different network boundary.

### Events, instructions and results

The client reports bounded normalized events with workspace, source connection,
provider event identity/cursor and observed time. The server validates the reporting
session, records provenance, deduplicates events and applies workspace rules. Client
reports are not independent proof of platform identity or moderator rights; the
platform remains the authority for actual provider permissions.

Instructions identify the workspace, authorized connection/sender/channel, action,
stable intent ID, registration generation, expiry and execution state. Multiple
clients must not execute the same intent concurrently. Collection ownership and
execution leases must prevent duplicate ingestion and duplicate bot actions.

The executor returns sent/rejected/failed/unknown outcomes, correlation IDs and
provider receipt IDs when available. A queued instruction is not a delivered
message. Reconnects and retries do not blindly repeat uncertain external writes;
the system does not claim exactly-once provider delivery. Instructions that expire
or lose their executor remain visibly unavailable/expired.

If no authorized client is connected, collection and external actions stop. The
server can keep rules, history and scheduling state, but cannot silently continue
platform operations. An always-on headless executor is an optional client role,
including when deployed on the same machine as the logic server. Its credentials
remain outside the logic-server process and are not forwarded through the bridge.

### MCP mapping

MCP is an adapter over this domain contract, not a source of credential authority.
For protocol revision 2026-07-28, clients initiate tool calls; a server cannot
arbitrarily call tools in the connected client. A suitable application-level
mapping lets the client register, publish events, retrieve pending instructions
and report results through typed MCP tools. Action batches may be returned directly
or retrieved by bounded cursor reads. This requires no public inbound listener on
the client. These application tools are proposed, not currently registered tools.

If client-local MCP tools are used for provider/AI execution, the executor acts as
their MCP host/client locally. The logic server returns validated domain intents;
it does not require unrestricted network access to those local tools. Supporting a
separate protocol revision or subscription transport requires explicit negotiation
and interoperability tests; an arbitrary command push is not assumed.

### Deployment modes

Independently operated installations use their own clients, bot accounts and AI
configuration. A later managed bot/AI offering can operate an executor client on
project-owned hardware and attach it to an authorized workspace. It remains
optional and does not move credentials into the user's Acheflow logic server.
Content reaches a remote/managed AI service only when explicitly configured.

## Consequences and migration

The earlier server-held bot model is superseded. Current manual token fields,
server OAuth handling, provider HTTP/WebSocket execution and server-routed chat
sends are legacy implementation awaiting migration. They must not be described as
implementing this boundary. Existing authenticated MCP state/command tools are a
starting point, not the completed registration/execution bridge.

Migration order and acceptance criteria are in
[the client execution specification](../SPEC-client-execution.md). In particular,
client OAuth, direct sends, workspace registration, bounded instruction retrieval,
results, cancellation and two independent providers must be verified before the
old server credential path is removed. Test fixtures do not prove live account
authorization. The optional managed service is future work.

## References

- [MCP architecture, 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/architecture)
- [MCP tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)
- [MCP transport message directions](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
