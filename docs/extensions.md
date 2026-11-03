# WASM extensions

The [v2 contract](extension-contract.md) and [ADR-002](decisions/002-client-execution-and-workspace-capabilities.md)
separate credential-free server logic from client-owned provider execution.
The server has no provider credential store, secret-write API or HTTP/WebSocket
handler. Provider manifests are `execution_side: "client"` and are skipped by server
discovery. Their cards come from the client catalog, independently of server status.
The shared browser/native page uses **Client** and **Server** tabs. Client contains
local integrations and identifies the current client/workspace. Server identifies
the selected endpoint and contains server logic plus workspace capabilities. Panels
stay mounted so switching tabs preserves drafts; keyboard arrows and Home/End move
selection and focus. If no local integrations are installed, Server opens initially. The webpage served from a server address is itself
a browser client: seeing its bundled local integrations does not install providers
in the logic-server process.
`acheflow-client` contains the native reactor and typed MCP adapter. See
[client executor setup and limitations](client-executor.md).

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

Install server logic directories containing `manifest.json` and a module beneath
`ACHEFLOW_EXTENSIONS_DIR`. Otherwise discovery uses `./extensions`, falling back
to `extensions` beside the server executable. Restart the server to discover new
installations. No provider list is compiled into the host. Only directories with
manifests are considered. Maximum: 8 installations, 64 directory entries inspected,
64 KiB manifest, 4 MiB module. Incompatible draft v1 manifests are rejected; build
v2 guests alongside the v2 host rather than silently reinterpreting the old ABI.

```json
{
  "id":"example", "name":"Example", "api_version":2, "module":"example.wasm",
  "execution_side":"client",
  "capabilities":["panel","http","status","channel","chat","delivery"],
  "services":["https://api.example.org"], "subscriptions":["chat.send"],
  "settings":[{"id":"application","label":"Application ID","max_length":128}]
}
```

Native provider discovery uses `ACHEFLOW_CLIENT_EXTENSIONS_DIR` when explicitly set;
otherwise it tries `./extensions`, then `extensions` beside the client executable,
then the client catalog bundled during compilation. An explicit unavailable directory
reports an installation error and does not silently fall back. Browser providers are
bundled automatically from client manifests in the guest workspace: after adding a
Rust guest/manifest and running `extensions/build.py`, rebuild the UI. No provider
names or protocol definitions are added to server/shared UI source.

Each workspace saves server logic under `extensions[id]` and local integrations under
`client_extensions[id]`, both with `enabled` and public `config`. The client migrates
known installed providers from the older shared map on load/import. Local settings
survive server-side merges and are omitted from Apply/Start; the server rejects any
nonempty client settings map submitted to it. See [persistence and conflicts](settings-persistence.md).
New/legacy entries default to disabled; disabled cards collapse and retain config.
Client toggles edit local workspace preferences without issuing server extension
commands. They do not grant server permissions or connect an account by themselves.
Server logic toggles retain their existing enabled-guest lifecycle and panel actions.
The account UI/runtime scheduling is still pending; connected executors can already
register separately through MCP. Policy and active capabilities are shown separately.

## Commands and events

The public Rust contracts are in `acheflow-extension-api/src/integration.rs`,
`interaction.rs` and `audience.rs`; UI/MCP actions are in
`acheflow-media-api/src/control.rs`. All host inputs carry `api_version`,
`workspace_id`, `extension_id`, `instance_id`, `request_id` and `now_ms`.

Guest inputs: `start`, one-second `tick`, panel `action`, `command`, subscribed
`event`, `http_result`, `websocket_open`, `websocket`, `websocket_closed`, `gap`.
`config` contains only this workspace's public settings. Network inputs and secret
handles are available exclusively in the client runtime. `storage` is private
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
Neither local storage nor credentials enter UI/MCP snapshots or exported workspaces.
The native runtime holds credentials only in memory, one workspace/account per instance;
there is no durable vault yet. Secret stores allow at most 32 handles of 8 KiB.
Credential replacement is local to the executor; dispose the old runtime and withdraw
its registration before connecting another account/generation.

The server accepts only declared public settings and rejects `secret: true` settings,
removed manual token fields and unknown extension configuration. There is no manual
Save on server / Remove credential UI, `set_extension_secret` action or
`configured_secrets` snapshot field. Client OAuth and local account panels are not
forwarded by the typed MCP adapter. Browser/native account UI integration is pending.

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
cargo test -p acheflow-client --locked
cargo test -p acheflow-control --locked
cargo test -p acheflow-server --locked
cargo test -p acheflow-ui --locked
```

Host integration tests load one real WASM fixture under independent provider IDs
and exercise fanout, receipts/echoes, partial failures, periodic/final polls,
donations, interactions, configuration, restart, cancellation and stale-instance
rejection. A boundary test rejects references to installed provider protocols in
production host/UI source. Provider-shaped fixtures do not prove live account access.


The client bridge currently forwards chat, audience, deletion and clear-chat events.
The shared `send_chat` command uses exclusive client instructions, with receipts and
merged echoes. Native provider poll commands, donations and provider interactions
have not yet been added to this bridge; they remain tested as pure server domain
logic and are not advertised as live client capabilities. Client runtime tests cover
real provider startup without authorization, two independent WASM fixtures through
real MCP, local identity/expiry/replay checks and cancellation of a real pending
transport connection. They do not prove live OAuth or provider access.
