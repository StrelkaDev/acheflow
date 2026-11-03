# Extension contract v2: provider-neutral host

Status: accepted by the project owner, 2026-11-02. Supersedes provider-specific
host bindings in SPEC-extensions.md and SPEC-twitch.md.

Authentication amendment, accepted 2026-11-03:
[ADR-002](decisions/002-client-execution-and-workspace-capabilities.md) moves all
integration/bot/OAuth and external AI credentials and platform I/O to clients.
The server receives normalized events, applies logic and returns typed instructions
for an authorized client to execute. Workspace capabilities may be configured
manually through MCP metadata or registered by a connected client, without secrets.
The current host credential and provider transport paths below are legacy behavior
awaiting migration; paired client/server runtimes are specified in
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

Manifest-declared secret fields are always captured/redacted regardless of the
individual request effect. The host supplies bounded HTTP/WebSocket I/O, ticks, secret handles and declarative
UI. HTTP results contain status and bounded JSON; captured secrets are redacted
before entering guest memory and are never included in workspace JSON/snapshots.
Credentials are scoped by workspace + extension; network calls remain restricted
to manifest origins, with no redirects. Guests use the same sandbox and lifecycle.

Settings marked `secret: true` are write-only server credentials. The shared host
and UI implement this once for every manifest; provider-specific field declarations
remain under `extensions/`. Manual writes use `set_extension_secret`, never public
config or guest events. The UI receives only configured handle names. Replacing or
removing a credential cancels the old guest before updating the scoped store and
restarting an enabled instance. Values are held in server memory until restart;
they are absent from workspace JSON, snapshots, panels and Debug output.

## Commands, events and lifecycle

UI/MCP -> host domain command -> enabled capable channel owner -> provider API.
Provider -> validated domain event -> host state -> shared native/browser UI.
Every delivery identifies workspace, extension instance, request/correlation ID,
channel and timestamp. Stop cancels I/O and rejects late instance effects. It
removes live channels and panels; saved configuration and historical data remain.
Subscribers receive only their declared command/event types. No arbitrary
provider-specific commands are introduced into the host API.

Channels declare capabilities. An empty target list means all available capable
channels; an explicit unsupported/unavailable target fails visibly. Chat fanout
creates one local message with per-target delivery status. Provider receipts and
echo IDs reconcile that message rather than adding duplicate local echoes.
Poll creation/closure is sent to the selected poll-capable channel owners. Each
provider publishes initial, periodic and final votes with its external poll ID;
the host aggregates counts and retains per-target errors/partial completion.
There is no hidden replacement of native polls by chat commands.
Interactions may be registered by users or extensions and trigger the same
validated host scene/action API. Donation providers publish normalized monetary
amounts as integer minor units plus currency; the host does not infer exchanges.

## Development and compatibility

The versioned manifest defines settings, subscriptions, capabilities and origins.
Adding a provider must require no production server/UI edit. Public configuration
belongs to workspace.extensions[id].config; legacy client_id migrates to config.
Draft ABI v1 is replaced explicitly by v2; build/install matching guest artifacts.
Rust guest sources compile to wasm32-unknown-unknown. WAT is only a sandbox test
fixture format, not the authoring workflow. Build guests before the host tests.

Required regression coverage: two different providers, generic settings/panels,
all/single chat dispatch and echo reconciliation, per-channel failures, poll
create/update/close, donation deduplication, interaction registration/trigger,
workspace isolation, stop/restart and late-event rejection, denied origins and
secret redaction. Live account testing is separate from provider-shaped fixtures.
