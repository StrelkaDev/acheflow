# Extension contract v2: provider-neutral host

Status: accepted; the governing contract for client integrations and server logic.

Credential ownership targets [ADR-004](decisions/004-extension-client-server-ownership.md),
which refines [ADR-003](decisions/003-trusted-server-workspace.md). Part selection
and separate runtime authority are implemented, including authorized ZIP install,
server-only shared collection, direct device actions and host-specific settings.
Final real-device qualification is tracked by SHARED-08 in
[the roadmap](roadmap.md#shared-server-workspace). Connecting or enabling a client
never implicitly transfers credentials.

## Ownership

Acheflow owns workspaces, channel discovery, merged chat, audience, donations,
polls, interactions, scenes, command routing and delivery state. Providers own
service URLs, OAuth scopes and flows, protocol parsing, subscription lifecycle,
rate-limit policy and mapping to/from the public domain API. There must be no
provider names, service URLs, OAuth response fields or transport implementations
in production server/UI code. Discover installations from manifests, not an
in-code list of providers. Provider code lives under extensions/.

## Package parts and compatibility

New packages use `api_version: 2`, `parts_version: 1` and an explicit `parts`
object. Declare `client`, `server`, or both; omitted parts are unavailable, never
substituted by the sibling. Only `id`, `name`, API/parts versions and `parts` belong
at package level. Mixing legacy runtime fields into this root is rejected.

```json
{
  "id": "example",
  "name": "Example integration",
  "api_version": 2,
  "parts_version": 1,
  "parts": {
    "client": {
      "kind": "provider",
      "module": "integration.wasm",
      "entry_point": "client_handle",
      "capabilities": ["http", "panel", "delivery"],
      "services": ["https://platform.example"],
      "supported_actions": ["chat.send"]
    },
    "server": {
      "kind": "provider",
      "module": "integration.wasm",
      "entry_point": "server_handle",
      "capabilities": ["http", "channel", "chat"],
      "services": ["https://platform.example"],
      "supported_actions": []
    }
  }
}
```

Each part independently declares `module`, `entry_point` (default `handle`),
`kind`, `capabilities`, `supported_actions`, `services`, `settings`, `secret_fields`,
`authentication`, `application_setup`, `forwarding`, `subscriptions`, `source_types`,
`chat_presentation` and `unique_viewers`. There is no root/sibling inheritance.
Service origins, settings and captured secret slots are validated separately.
Actions are bounded to `chat.send`, `chat.delete`, `user.timeout`, `poll.create`
and `poll.close`; declaration alone does not implement an action, authorize an
account or grant platform rights. The native provider instruction path rejects
personal writes absent from the selected part's declared actions. Channel/domain
capabilities still describe current availability and permissions.

A client part uses `kind: "provider"` (the local reactor interface), including for
local features without network capabilities. A server part uses `provider` for
the explicit trusted reactor or `logic` for the ordinary credential-free domain
host. Logic parts cannot request transport, credentials, OAuth setup or callback
forwarding. Visual source declarations remain server-logic functionality.

Loaders call `GuestManifest::select_part(ExecutionHost)` before opening the module,
settings or credentials. The resolved manifest contains only that part, records
its host in memory and cannot be reselected for another host. Server logic and
trusted providers have separate catalogs. A missing declared WASM export fails
loading; module paths remain bounded local filenames inside the installation.
`ProviderRuntime::new` opens client authority; `new_for_host` selects explicit host
authority. Local sessions and server account installation reject cross-host use.
Credentials/private storage remain host-owned: part names cannot choose another
host's vault namespace. Existing workspace and account fencing still applies.

The ABI remains JSON through `acheflow` imports and a no-argument WASM export.
`guest_sdk::export_part!(client_handle, State)` and the corresponding server export
create independent state; a package may use the same artifact or separate artifacts.
The host overwrites `execution_host` on every provider/auth turn; SDK authorization
selects the matching embedded manifest part. Never infer host authority from a
caller-supplied action or account identifier.

Legacy single-side manifests without `parts`/`parts_version` remain readable:
`execution_side: "server"` (the default) means pure server logic; `client` means
the provider interface. Only the explicit trusted server provider loader continues
to admit legacy client manifests during migration. New client-only packages are
not admitted there. Legacy actions omitted from manifests retain their existing
runtime checks. Unknown parts versions and mixed declarations fail closed. Older
hosts reject new package fields; update the host and rebuild guests together.

Bundling, guest builds and macOS packaging include declared part artifacts and
browser authorization modules. Generated WASM stays ignored and untracked. See [package installation](extensions.md#install-packages) for host approval and
client/browser limitations.

Both provider hosts use the bounded native reactor, opaque handles and declared
service origins with no redirects. Server bindings reserve collection authority;
opening a second client cannot create another subscriber for that channel. Server
snapshots distinguish provider execution status from pure logic extensions. Public
workspace settings contain binding references and grants, never credentials.
Server accounts live in the server's own workspace vault; personal accounts remain exclusively in
the originating device vault/session. A server authorizer uses a separate temporary
session and cannot open the device credential container.
Local auth panels and token exchange responses never enter server
snapshots or MCP events. Client account panels and manual secret inputs are emitted
by provider WASM guests; shared UI only renders their generic controls. OAuth flow,
permissions, validation and renewal stay under `extensions/`. Committed native Mac
credentials use the client's system vault; browser credentials remain session-local.
See [local authorization](extension-authorization.md) and the remaining executor
boundary in [client-executor.md](client-executor.md).

### Visual source documents

Server logic manifests may declare `source_types` with a local `id`, display
`name` and public `settings`, and request the `source` capability. Source instances
are workspace sources with `kind.type: "extension"`, an `extension_id`, `type_id`
and public settings. Each guest turn includes only that extension's `sources`.
Adding a source type requires no provider changes in server or UI code. The sample
extension's Action counter is a working example.

An enabled live guest publishes `{"type":"source","frame":{...}}`. The frame
contains `source_id`, `type_id` and `document` (`css`, `nodes`). The host supplies
the extension identity and rejects undeclared types, another extension's source,
inactive instances and late updates from replaced instances. Disabling the guest
withdraws its documents. Workspace changes discard the previous source state.

Nodes support only `div`, `span`, `strong` and `p`, bounded text, CSS classes and
children. Text is escaped. No scripts, raw HTML, attributes or external assets
are accepted. Documents are limited to 512 nodes, eight nesting levels and 192 KiB
of text; source CSS uses the same resource-free validator as layer CSS. Optional
`animation_at_ms`, `exit_at_ms` and `remove_at_ms` define absolute phase times;
the shared renderer applies `.entering`/`.leaving` and removes expired nodes using
the scene clock. Updates preserve existing animation timing.

The built-in Chat source consumes normalized merged chat through this same visual
document renderer; provider guests continue publishing domain chat events. It has
no transport credentials or provider-specific scene code. See
[live production](live-pipeline.md) for source editing.

### Native streaming transport

Client manifests may declare the `http_stream` capability; server manifests reject
it. The native runtime supports one owned streaming connection alongside its
WebSocket. An open effect contains `type: "http_stream"`, a bounded `id`,
`protocol: "grpc"`, a manifest-granted HTTPS `url`, optional secret-handle `headers`
and `body_base64` containing the provider's protobuf request (up to 64 KiB).
The host owns HTTP/2/TLS framing, certificate validation, a thirty-second opening
deadline and cancellation. Streaming services can defer initial response headers
until the first batch. It follows no redirects.
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
are suppressed. Failed opens may include a fixed `transport_error` category:
`network`, `tls`, `http2`, `request`, `timeout`, `protocol` or `invalid_request`.
These identify local failure stages without exposing raw errors or credentials.
The native host accepts protobuf gRPC content types with optional media-type
parameters, while rejecting gRPC-Web and non-protobuf subtypes.
A missing/truncated/compressed/oversized frame or missing final
gRPC status closes the stream with a generic protocol failure. A guest's close
effect (`http_stream { id, close: true }`), replacement stream or runtime drop
destroys its response body and aborts its HTTP/2 driver. Runtime receive timestamps
include elapsed waiting time before provider backoff is calculated. This is a
native library transport, not a browser gRPC shim. The selected native client or trusted server owns continuous runtime scheduling.
The wire framing follows the
[gRPC HTTP/2 protocol](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md).

## Commands, events and lifecycle

Manifests may declare `forwarding` actions (`id`, `mode: event|callback`), up to 16
unique action IDs. Undeclared actions are rejected. Client callbacks require a
capability-checked `forward_wait` effect bound to a host-issued crypto nonce,
workspace and live instance; accepted callbacks are consumed once. Declared secret
fields are captured before local guest delivery. `forward_wait` may bind an exact
opaque `tail`; wrong-tail responses leave the valid request pending. Declared secret
fields never enter server payloads. Server guests support event
forwarding only, through private bounded per-instance queues. Forwarding does not
activate disabled guests or grant provider I/O to server logic. See
[forwarding addresses and delivery](forwarding.md).

A client guest can emit `channel { reset: true }` to withdraw its current local
channel before discovery/selection changes. This uses its declared `channel`
capability. The Studio executor unregisters the previous generation; late stream
events and pending messages must be discarded by the provider. Registration and
publication remain subject to the saved workspace policy.

Panels can use generic `row` and `disclosure` widgets to group controls. A button's
optional `primary` flag defaults to false. Layout stays guest-declared and the host
renders the same wrapping rows and keyboard-operated disclosures on both targets.
The 64-widget limit includes nested children; layout nesting is limited to 8 levels.
Local buttons can declare `requires` with up to 16 unique setting/credential IDs.
The host disables mouse and keyboard submission until every declared input is
present, within its length limit and without whitespace. Credential drafts are
checked locally without persisting or exposing their values. Saved credentials
are tested for presence only; unknown IDs keep the button disabled. These local
requirements cannot enable actions on a server panel. Guests still validate actions.

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

Application setup fields are visible before Connect, including required native
application secrets. Registration instructions, console links and callback/origin
addresses stay in a disclosure. Application identifiers have no embedded defaults:
users select their own registered application and its quota. Secrets remain local
credential inputs; browser setup cannot request application secrets. After successful
authorization, a visible Disconnect replaces Connect and clears saved credentials
and pending callbacks. The host remains provider-neutral.

The local host supplies `environment.link` from the installed manifest ID as
`https://app.acheflow.com/fwd/<id>`, replacing caller-provided link bases. An
extension may declare `authentication.native_callback: "oauth/callback"`; the
SDK validates the installed route against this declaration. The host supplies
`environment.app_callback_uri` from its separately configured client/server
callback proxy URL. With this transport the guest emits
`browser { callback_uri, activation_uri, ... }` and a versioned callback envelope
in the correlation field; `activation_uri` is the declared installed route.
The native host verifies both the configured proxy and declared activation route
before opening the browser. The static proxy only restores the opaque nonce and
selects native versus browser delivery. The waiting client still checks exact
path, nonce, workspace, live instance and expiry. It never trusts a workspace
selection supplied by the proxy. Without the shared transport, legacy declared
app callbacks and owned ephemeral loopback listeners remain supported. Browser
SDK modules retain their own transport; direct browser callbacks remain same-origin.
The static envelope is not OAuth-specific: `/callback/` and `/callback/*`
serve the same router, and extensions may use a declared correlation field other
than `state`. Contextual paths do not select a workspace or authorize a request. No provider URL, credential or permission mapping belongs in shared
host code.

UI/MCP -> server domain command -> outbox -> client claim -> provider API -> receipt.
Provider -> client normalized typed event -> server logic -> shared native/browser UI.
Every delivery identifies workspace, extension instance, request/correlation ID,
channel and timestamp. Stop cancels I/O and rejects late instance effects. It
removes live channels and panels; saved configuration and historical data remain.
Subscribers receive only their declared command/event types. No arbitrary
provider-specific commands are introduced into the host API.

Manifests may declare optional `chat_presentation` metadata: `icon_path` is a
bounded SVG path in a 24×24 view box (no markup, scripts or URLs), and `help`
is plain tooltip text. The shared UI renders this metadata with channel labels
and capabilities; unknown extensions receive a generic chat icon.

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

## Trusted-server account validation

Provider guests supporting explicit server connection handle
`local_server_authorize` after `local_start`. The host stages only declared secret
slots and supplies public `config`, presence-only `secrets` and `now_ms`; it never
accepts a client-supplied storage object. The event must perform platform validation
through extension-owned HTTPS effects and commit credentials with `secret_commit`
and verified `storage.account`. An uncommitted/missing/empty account is rejected.
The shared authorization SDK implements this event using the manifest validation
request and identity/application/scope checks. Non-supporting packages fail closed.

This validation instance cannot open a browser or publish channels/chat, launch
subscriptions or execute commands. The host accepts only bounded panel/status and
HTTPS validation effects and suppresses arbitrary guest/platform errors in its API
response. A separate server runtime starts only after validation and durable save.
New server connections grant only `chat.read`; sending requires separate policy.
See [the connection flow](extension-authorization.md#connect-a-trusted-server-account)
for trust, migration and disconnect behavior.

## Device-local actions

Client sender parts declare `chat.send` in both subscriptions and supported actions.
They authorize the sender without opening read subscriptions. The action host
validates the committed account again, then calls `prepare_action {channel_id}`.
Preparation permits only bounded GET/HEAD metadata requests, private storage and
channel discovery; it rejects subscriptions and writes. It must resolve the exact
shared target and advertise `chat.send`. Cleanup-only transport closes are allowed.
The host calls a fresh server permission check immediately before one bounded
external write, then dispatches typed `command` with a correlated request ID.
No collector registration, server personal secret storage or server-account fallback
participates. Lost/uncertain receipts are terminal; never repeat the write. Future
moderation needs separately declared actions and platform checks.
