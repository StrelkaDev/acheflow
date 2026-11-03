# WASM extensions

The [v2 architecture contract](extension-contract.md) is the governing boundary.
The server owns domain state and routing; provider protocols and OAuth live entirely
inside independently compiled Rust guests. Native and browser UIs render the same
manifest settings and guest panels. There is no provider-specific host binding.

[ADR-002](decisions/002-client-execution-and-workspace-capabilities.md) defines the
accepted target: clients own all integration credentials, platform collection and
execution; the server processes normalized events and returns typed instructions.
Manual MCP configuration and automatic client registration feed one workspace
capability registry while saved policy remains separate from live availability.
The [execution specification](SPEC-client-execution.md) records the migration.
The current code still uses server-held tokens, provider transport and server-routed
sends, including manual token entry for YouTube/VK. Client-only integration execution
has not been implemented; details below document the existing host runtime.

## Build and installation

```sh
rustup target add wasm32-unknown-unknown
python3 extensions/build.py
cargo run -p acheflow-server --locked
```

`extensions/` is a separate Cargo workspace. `guest-sdk` wraps the checked linear
memory ABI; `sample/src/lib.rs` is a minimal authoring example and
`twitch/src/lib.rs` contains the complete first provider. The build script installs
matching `.wasm` artifacts beside their manifests. WAT remains supported by wasmi
for low-level sandbox tests, but provider sources are Rust.

Install a directory containing `manifest.json` and its module beneath
`ACHEFLOW_EXTENSIONS_DIR`. Otherwise discovery uses `./extensions`, falling back
to `extensions` beside the server executable. Restart the server to discover new
installations. No provider list is compiled into the host. Only directories with
manifests are considered. Maximum: 8 installations, 64 directory entries inspected,
64 KiB manifest, 4 MiB module. Incompatible draft v1 manifests are rejected; build
v2 guests alongside the v2 host rather than silently reinterpreting the old ABI.

```json
{
  "id":"example", "name":"Example", "api_version":2, "module":"example.wasm",
  "capabilities":["panel","http","status","channel","chat","delivery"],
  "services":["https://api.example.org"], "subscriptions":["chat.send"],
  "settings":[{"id":"application","label":"Application ID","max_length":128}]
}
```

Each workspace saves `extensions[id].enabled` and the public `config` map. Old
`client_id` values migrate into `config.client_id`. New/legacy workspaces default
to disabled. Applying/starting restores enabled guests; switching/deactivating
stops the previous instances. Reapply preserves running guests. Disabled cards
collapse, retain configuration and remove live panels/channels. Errors stay inside
the owning card, including errors enabling an otherwise collapsed extension.

## Commands and events

The public Rust contracts are in `acheflow-extension-api/src/integration.rs`,
`interaction.rs` and `audience.rs`; UI/MCP actions are in
`acheflow-media-api/src/control.rs`. All host inputs carry `api_version`,
`workspace_id`, `extension_id`, `instance_id`, `request_id` and `now_ms`.

Guest inputs: `start`, one-second `tick`, panel `action`, `command`, subscribed
`event`, `http_result`, `websocket_open`, `websocket`, `websocket_closed`, `gap`.
`config` contains only this workspace's public settings. `storage` is private
per-extension session metadata; `secrets` lists available opaque handle names.
Only scene-capable guests receive a scene/revision snapshot.

| Host command name | Routed payload |
|---|---|
| `chat.send` | `channel_id`, `text`; `request_id` identifies the send intent |
| `poll.create` | `channel_id`, `poll_id`, `question`, ordered `options`, `duration_seconds` |
| `poll.close` | `channel_id`, `poll_id`, `cancel` |

Guests publish capability-checked effects:

| Effect `type` | Payload |
|---|---|
| `channel` | `channel: {id,label,capabilities,constraints?}`; host supplies `extension_id` |
| `chat` | `message: ChatMessage` with provider message ID and normalized roles |
| `audience` | `sample: AudienceSample`, including optional scoped unique metric |
| `delete` | `platform`, `channel_id`, `message_id`, `all` |
| `delivery` | `request_id`, `channel_id`, `sent: true/false/null`, `external_id`, `error` |
| `poll` | `update: {poll_id,channel_id,external_id,state,votes,error}` |
| `donation` | `donation: {id,channel_id,display_name,amount_minor,currency,decimal_places?,message,occurred_at_ms}` |
| `interaction` | `action: register`, `interaction: {id,label,enabled,action?}`; or `action: trigger`, `id`, unique `event_id` |
| `status` | `state`, optional public `account` and `error` |
| `panel` | `panel: {extension_id,title,widgets}` |
| `failure` | bounded human-readable `message` |
| `scene` | `scene_id`, full `spatial`, snapshot `expected_revision`, stable `request_id` |

Channel capability names are `chat.send`, `poll.create`, `interaction.triggered`.
A channel may advertise constraints such as `chat.max_chars`, `poll.max_options`,
`poll.min_duration_seconds`, `poll.max_duration_seconds`, `poll.max_question_chars`
and `poll.max_option_chars`; the host validates selected channels before dispatch.
Read subscriptions use `read.chat`, `read.audience`, `read.poll`, `read.donation`
or `read.interaction`, in addition to the explicit topic subscription. Domain
notifications include `chat.message`, `chat.deleted`, `chat.delivery`,
`audience.updated`, `poll.updated`, `donation.received`, `interaction.registered`, `interaction.triggered`
and `channel.updated`. Guests never consume their own emitted notification.

The channel registry drives UI selectors. `send_chat.targets=[]` selects all live
chat-capable channels; otherwise use their qualified keys `extension_id:local_id`.
One outgoing message contains per-target `pending/sent/failed/unknown` deliveries.
Receipts reconcile provider echo IDs. Cursor reads publish updated delivery rows
and echo tombstones (`merged_into`); incremental clients merge by message identity
and remove entries marked as merged. Full snapshots omit those tombstones. Retrying the same command intent does not
resend. An uncertain network outcome is never automatically retried as a send.

Polls use actual provider polls. Providers retain external IDs, periodically
publish votes and publish a final result after `poll.close` or native expiry.
`active/completed/failed` updates apply only to targeted channels of that provider;
completed targets are immutable. The host sums votes and preserves each target's
state/error. Missing final results show a visible warning after a 30-second grace, while
known external IDs remain available for recovery and closure.
It never substitutes chat voting. Interactions are namespaced; a user may register
an Acheflow interaction or bind an extension-declared interaction to a validated
scene/object rotation. Replayed trigger IDs do not repeat the action.

## Transport, credentials and sandbox

`http` effects declare `id`, `url`, optional `method`, `headers`, `query`, `body`
or `form`, and optional `capture` mapping secret names to JSON pointers. The result
contains `id`, numeric `status`, bounded `payload`, `retry_after`; transport errors
use status zero. Protocol-specific status handling/backoff belongs to the guest.
Manifest `secret_fields` maps handle names to JSON pointers and is always applied
to every response, even if a guest omits request-level capture. Additional request
captures may be declared for one operation. Captured string fields are stored by workspace + extension and replaced by null
before delivery. Example: `"capture":{"access":"/access_token"}`. Request values
may reference `{"secret":"access","prefix":"Bearer "}`; substitution happens
only in the host after origin validation. Do not put credentials in config/panels.

`websocket` opens an allowed WSS URL or closes with `close:true`. The host transports
JSON and answers protocol pings. `send` transmits JSON on the existing socket and
returns `websocket_sent` (transport acceptance, not provider delivery); optional
handshake headers and JSON messages support opaque secret references. Requests,
including substitution, are bounded before allocation; reserved transport headers
cannot be overridden. Connections/sends have ten-second deadlines. Service welcome/reconnect/keepalive messages are
interpreted by the guest. No redirects, HTTP, userinfo URLs or ungranted origins.
`storage` stores a JSON `value` up to 64 KiB for this workspace/extension/session.
Neither storage nor secrets enter UI/MCP snapshots or exported workspaces. Server
restart requires account authorization again. Each store has at most 256 namespaces;
secret stores allow at most 32 handles of 8 KiB per namespace.

A manifest setting may declare `"secret": true`. The shared UI renders a masked,
initially empty input with explicit Save on server / Remove controls. Saving uses
the write-only `set_extension_secret` control action (`id`, `key`, `value`, optional
public `config`); an empty value removes that handle. Undeclared fields and secrets
in public config are rejected. Snapshots expose `configured_secrets` handle names
only. Manual updates stop the old guest before changing credentials, then restart
it if enabled, so late requests cannot restore an old token. Debug formatting
redacts the secret-bearing command value. Credentials remain in server memory until
restart. This manual-input action is not advertised by the MCP command schema.

wasmi 2.0.0 limits each call to 10 million fuel, 16 MiB linear memory, 1 MiB
input/output and 64 effects. Host JSON/copy/template work also consumes fuel. Queues
are bounded at 512; cursor gaps become visible errors. Stop cancels pending I/O,
drops subscriptions and rejects late instance publications. Streaming survives
extension failure. Guest HTML/JavaScript, filesystem access and media/GPU objects
are outside the ABI. Panels support labels, metrics, buttons, text inputs, choices
and HTTPS links restricted to declared service origins.

Export `memory` and `handle: () -> ()`; import from `acheflow`: `input_len() -> i32`,
`read_input(ptr:i32,capacity:i32) -> i32`, `emit_json(ptr:i32,len:i32)`. The Rust SDK
encapsulates pointer operations. Legacy low-level `matches`/`emit` template imports
remain testable, but are not the provider authoring path.

## Required verification

```sh
python3 extensions/build.py
cargo test --manifest-path extensions/Cargo.toml --locked
cargo test -p acheflow-control --locked
cargo test -p acheflow-server --locked
cargo test -p acheflow-ui --locked
```

Host integration tests load one real WASM fixture under independent provider IDs
and exercise fanout, receipts/echoes, partial failures, periodic/final polls,
donations, interactions, configuration, restart, cancellation and stale-instance
rejection. A boundary test rejects references to installed provider protocols in
production host/UI source. Provider-shaped fixtures do not prove live account access.
