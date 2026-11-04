# Extension contract v2: provider-neutral host

Status: accepted by the project owner, 2026-11-02. Supersedes provider-specific
host bindings in SPEC-extensions.md and SPEC-twitch.md.

Authentication amendment, accepted 2026-11-03:
[ADR-002](decisions/002-client-execution-and-workspace-capabilities.md) moves all
integration/bot/OAuth and external AI credentials and platform I/O to clients.
The server receives normalized events, applies logic and returns typed instructions
for an authorized client to execute. Workspace capabilities may be configured
manually through MCP metadata or registered by a connected client, without secrets.
Server credential storage and provider transports have been removed. Execution is specified in
[the execution specification](SPEC-client-execution.md). A later managed executor/AI
service is optional and acts as a client, not a credential store in the logic server.

## Ownership

Acheflow owns workspaces, channel discovery, merged chat, audience, donations,
polls, interactions, scenes, command routing and delivery state. Providers own
service URLs, OAuth scopes and flows, protocol parsing, subscription lifecycle,
rate-limit policy and mapping to/from the public domain API. There must be no
provider names, service URLs, OAuth response fields or transport implementations
in production server/UI code. Discover installations from manifests, not an
in-code list of providers. Provider code lives under extensions/.

Manifests declare `execution_side: "server"` or `"client"`. The default is `server`,
which rejects service origins, secret captures/settings and HTTP/WebSocket capabilities.
There is no compatibility execution mode. A server logic guest may handle pure domain
rules, bounded private noncredential storage and panels, without platform I/O.

The server skips client manifests entirely. The client owns provider discovery:
native clients read their installation directory (or the bundled fallback), while
browser clients use the manifest/WASM catalog bundled at UI build time. Server
snapshots list server logic only; live client connections are separate metadata.
Client providers need no server installation to register authorized capabilities.
A native client runtime supplies bounded HTTP/WebSocket I/O and opaque secret handles
for these guests. Manifest/effect captures redact credentials before guest delivery.
Secrets remain local to one workspace/account runtime; network calls are restricted
to declared origins with no redirects. OAuth/protocol definitions remain under
`extensions/`. Client auth panels and token exchange responses never enter server
snapshots or MCP events. Client account panels and manual secret inputs are emitted
by provider WASM guests; shared UI only renders their generic controls. OAuth flow,
permissions, validation and renewal stay under `extensions/`. Committed native Mac
credentials use the client's system vault; browser credentials remain session-local.
See [local authorization](extension-authorization.md) and the remaining executor
boundary in [SPEC-client-execution.md](SPEC-client-execution.md).

## Commands, events and lifecycle

Manifests may declare `forwarding` actions (`id`, `mode: event|callback`), up to 16
unique action IDs. Undeclared actions are rejected. Client callbacks require a
capability-checked `forward_wait` effect bound to a host-issued crypto nonce,
workspace and live instance; accepted callbacks are consumed once. Declared secret
fields are captured before local guest delivery. Server guests support event
forwarding only, through private bounded per-instance queues. Forwarding does not
activate disabled guests or grant provider I/O to server logic. See
[forwarding addresses and delivery](forwarding.md).

Panels can use generic `row` and `disclosure` widgets to group controls. A button's
optional `primary` flag defaults to false. Layout stays guest-declared and the host
renders the same wrapping rows and keyboard-operated disclosures on both targets.
The 64-widget limit includes nested children; layout nesting is limited to 8 levels.

UI/MCP -> server domain command -> outbox -> client claim -> provider API -> receipt.
Provider -> client normalized typed event -> server logic -> shared native/browser UI.
Every delivery identifies workspace, extension instance, request/correlation ID,
channel and timestamp. Stop cancels I/O and rejects late instance effects. It
removes live channels and panels; saved configuration and historical data remain.
Subscribers receive only their declared command/event types. No arbitrary
provider-specific commands are introduced into the host API.

Channels declare capabilities. An empty target list means all available capable
channels; an explicit unsupported/unavailable target fails visibly. Chat fanout
creates one local message with per-target delivery status. Provider receipts and
echo IDs reconcile that message rather than adding duplicate local echoes.
Native poll routing remains available for pure logic fixtures; client provider poll
instructions have not yet been added and are not advertised as available. Each
provider publishes initial, periodic and final votes with its external poll ID;
the host aggregates counts and retains per-target errors/partial completion.
There is no hidden replacement of native polls by chat commands.
Interactions may be registered by users or extensions and trigger the same
validated host scene/action API. Donation providers publish normalized monetary
amounts as integer minor units plus currency; the host does not infer exchanges.

## Development and compatibility

The versioned manifest defines settings, subscriptions, capabilities and origins.
Adding a provider must require no production server/UI edit. Public configuration
belongs to `workspace.extensions[id].config` for server logic and
`workspace.client_extensions[id].config` for client integrations. Apply/Start use a
server projection that excludes client settings. Known installed client providers
migrate from the older shared settings map on client load/import, preserving newer
client-scoped values when both formats exist. Legacy client_id migrates to config.
Draft ABI v1 is replaced explicitly by v2; build/install matching guest artifacts.
Rust guest sources compile to wasm32-unknown-unknown. WAT is only a sandbox test
fixture format, not the authoring workflow. Build guests before the host tests.

Required regression coverage: two different providers, generic settings/panels,
all/single chat dispatch and echo reconciliation, per-channel failures, poll
create/update/close, donation deduplication, interaction registration/trigger,
workspace isolation, stop/restart and late-event rejection, denied origins and
secret redaction. Live account testing is separate from provider-shaped fixtures.
