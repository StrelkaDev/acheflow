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

### Native streaming transport

Client manifests may declare the `http_stream` capability; server manifests reject
it. The native runtime supports one owned streaming connection alongside its
WebSocket. An open effect contains `type: "http_stream"`, a bounded `id`,
`protocol: "grpc"`, a manifest-granted HTTPS `url`, optional secret-handle `headers`
and `body_base64` containing the provider's protobuf request (up to 64 KiB).
The host owns HTTP/2/TLS framing, certificate validation, a ten-second connection
deadline and cancellation. It follows no redirects.
HTTP/2 uses fixed 64-KiB receive windows and 16-KiB frame/header limits. Guests cannot override Host,
framing/connection headers, Content-Type, TE or reserved `grpc-*` headers. Query
parameters and credential-capture mappings are not supported on this effect;
OAuth exchanges continue through bounded JSON HTTP and host credential capture.
Protobuf field numbers, service/method paths, subscriptions and retries remain
inside the extension. The host neither interprets provider protobuf nor forwards
raw streaming data through MCP.

After `http_stream_open { id }`, the guest requests the next piece with
`http_stream { id, read: true }`. It receives `http_stream_data { id, data,
end_message }`: base64-encoded pieces of at most 8 KiB from an uncompressed gRPC
message, whose complete size is limited to 512 KiB. The host retains partial
headers/body pieces across cancellation of a pending receive. It waits for another
explicit read before advancing, so a guest can drain its batch using ordinary
ticks without an unbounded queue or one oversized WASM call.

`http_stream_closed` carries only the stream ID, HTTP status, numeric gRPC status
and optional bounded `retry_after_ms`; provider diagnostic strings/trailer details
are suppressed. A missing/truncated/compressed/oversized frame or missing final
gRPC status closes the stream with a generic protocol failure. A guest's close
effect (`http_stream { id, close: true }`), replacement stream or runtime drop
destroys its response body and aborts its HTTP/2 driver. Runtime receive timestamps
include elapsed waiting time before provider backoff is calculated. This is a
native library transport, not a browser gRPC shim or an always-on UI executor.
The wire framing follows the
[gRPC HTTP/2 protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md).

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

Client manifests may declare typed `application_setup` metadata from
`acheflow-extension-api::application_setup`: developer-console HTTPS links,
native/browser instructions, an optional active `callback_uri` or `origin` address,
public `settings` IDs and required native `secrets` IDs. References resolve only
against that installation's declared settings and secret slots; duplicates,
unknown IDs, browser application secrets and unbounded metadata are rejected.
Setup uses the discovered catalog and the workspace's enabled installations.
The guest emits `setting_input { id }` for public settings and
`copy_text { label, value }` for a read-only public address with a copy action.
The host binds public fields by extension/setting ID, retaining the manifest's
labels, limits and local persistence. Copy values are limited to 8192 bytes;
clipboard errors stay inside the control. Device flows can omit the address.
OAuth endpoints, scopes and flow decisions remain extension-owned; setup does
not mandate hosted callbacks or change the selected authorization transport.

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
