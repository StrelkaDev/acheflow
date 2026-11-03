# Spec: mcp

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `mcp` in the
[capability map](../CAPABILITY-MAP.md).

Architecture amendment, 2026-11-03:
[ADR-002](decisions/002-client-execution-and-workspace-capabilities.md) adds the target
of client-owned credentials/execution and a workspace capability registry populated
manually or by a connected client. The [execution specification](SPEC-client-execution.md)
defines the proposed registration/event/instruction/result bridge. The current
state/command MCP adapter below does not yet implement that bridge. On MCP
2026-07-28, instructions are returned to client-initiated calls; the server does not
initiate arbitrary tool calls on a client. Legacy server-routed provider sends must
be migrated to client execution before claiming this architecture is implemented.

## Objective

Let an external MCP client inspect chat, audience metrics, polls, extensions and
the active scene, and execute authorized application actions. MCP is an adapter
over the same services as the UI and extensions, not a separate source of state.

## Contracts and structure

Put MCP transport/protocol mapping in a separate server module or thin executable
adapter, selected in the plan. Keep domain types and behavior in their provider
modules. Use a maintained protocol SDK verified against the actual target clients;
pin supported protocol revisions and test compatibility explicitly. The current
official transport reference is version 2026-07-28; do not assume an older handshake
or session model works unchanged.

First-release transport: Streamable HTTP on a dedicated authenticated local
endpoint. Bind it to loopback by default, validate Origin and authorize connections;
do not accidentally inherit an all-interface media-server listener or permissive
UI CORS policy. Remote/public hosting and a stdio adapter are subsequent deployment
options. Document a concrete client configuration and credential setup in the plan.

Expose typed reads for workspace/scene/camera/objects, extension status, chat pages,
poll state and analytics. Chat/event/history reads use bounded cursor pagination;
snapshots carry revision, event cursor, freshness/coverage and optional metric
availability. A client can consume changes through supported MCP subscription
features or cursor reads; no unbounded tool call is required to read recent events.

Tools cover chat send, poll create/close/cancel, registered command management,
extension enable/disable/restart and scene/object/camera/viewport mutations. Provider
specs own each input/output schema. Every call checks actor capabilities, workspace
identity, typed arguments and expected revision where relevant. Read access does
not imply permission to send chat, alter scene or manage extensions. Exclude token,
stream-key, arbitrary shell, file/network and raw credential-export tools.

Map domain errors to structured tool results with stable codes and bounded
explanations; keep JSON-RPC/protocol errors distinct. Mutations carry a stable
request id. Atomically reject concurrent duplicates, reject reuse with different
arguments and replay completed local results while retained. Define retention in
the plan and reject expired retries explicitly. External send timeouts remain
`outcome_unknown`; do not promise exactly-once provider delivery. Service timeouts
and cancellation cannot leave hidden unbounded subscriptions or guest work.

Chat is externally supplied text in all reads. Reading a message never executes
it, and scene tools cannot bypass configured command permissions. Responses do not
silently expand user capabilities based on text found in chat or extension panels.

## Acceptance and testing

- A real external MCP client reads chat and analytics, runs a poll and changes a
  scene object; native/web editor and encoded output show the same state.
- Protocol tests cover the supported revision(s), transport metadata, tool schemas,
  pagination/cursor gaps, cancellation and structured domain failures.
- Unauthorized connections/origins and read-only callers cannot mutate state.
  Cross-workspace ids, stale revisions and repeated request ids behave as specified.
- Repeated/timeout external sends do not become automatic duplicate chat messages;
  unavailable unique-viewer data and partial totals retain analytics semantics.

Commands: `cargo test -p acheflow-server --locked`, the shared implementation
checks, and client interoperability tests whose exact commands are added once
the SDK/client setup is selected. Follow shared typed serde/error conventions.

Always reuse application handlers and use official protocol behavior for the
selected revision. MCP-client platform integrations, public hosting and arbitrary
agent execution inside Acheflow are outside this release.

Reference: [MCP standard transports, 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
and its [Streamable HTTP binding](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http).
