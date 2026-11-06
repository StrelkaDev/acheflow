# Architecture

Acheflow uses a shared Rust workspace for desktop/browser UI, a headless media and
logic server, and typed application contracts. Rust/WASM guests live in a separate
Cargo workspace under `extensions/`. One production executable selects desktop,
server or link dispatch without introducing provider-specific host code.

## Packages

| Package | Responsibility |
| --- | --- |
| `acheflow` | Production desktop/server entrypoint and recipient dispatch |
| `acheflow-ui` | Shared Dioxus components, CSS, native/browser rendering and local workspace UI |
| `acheflow-client` | Provider discovery, local authorization/runtime, credential handling, executor bridge and owned server lifecycle |
| `acheflow-server` | HTTP/RTMP listeners, media workers, server logic, control and MCP adapters |
| `acheflow-launcher` | Bounded native activation, private per-recipient IPC and instance election |
| `acheflow-composition` | Shared scene geometry, CSS/WGSL composition, 3D viewports and GPU readback |
| `acheflow-media-api` | Workspace/scene/audio types, validation, media status and control DTOs |
| `acheflow-extension-api` | Versioned manifests, panels, events, forwarding and client execution contracts |
| `acheflow-control` | Provider-neutral interaction, analytics, metered guests and execution registry |

The older `acheflow-ui` and `acheflow-server` Cargo binaries are development
entrypoints into shared libraries. Distribution packages only the unified primary
executable, resources and native runtime libraries.

## Startup and native activation

The entrypoint validates arguments and selects an explicit mode before UI
initialization. `--server`, help and version do not construct a native event loop
or studio. On macOS an OS launch creates the AppKit event loop and installs the
main application's delegate first. Launch intent and incoming URLs select the
recipient; only client mode constructs the Dioxus tree, renderer, window and local
server manager. A server URL uses the same executable's server runtime with no UI.
Nondefault launches wait for their activation even if macOS delivers it after the
launch-completion notification; the wait has a bounded timeout. AppKit requests
for an untitled window or reopen use the same Show path and preserve an explicit
launch URL.

The delegate remains installed for warm scheme and supported HTTPS events.
Bounded background delivery keeps synchronous local IPC off the UI thread. Client
and server retain separate instance election and lifetimes; a missing recipient
can run as another process of the same executable. The macOS package needs no
separate event-helper executable. App termination cancels the in-process server
and joins its runtime before exit.

## Media production

```text
RTMP publisher -> source-token validation -> FLV demultiplexing
                                         -> native-resolution video decode
                                         -> independent audio-track decode
video -> shared GPU scene compositor -> RGB readback -> per-output FFmpeg -> RTMP
                                     -> JPEG snapshots -> desktop/browser previews
audio -> scene strips and stereo buses ----------------> per-output AAC
```

FFmpeg is a managed subprocess, not a library linked into the UI. Sources retain
native decoded dimensions; scene layout handles fit/crop/scaling. Each destination
owns its encoder/output lifetime. Audio mixes are scene-scoped and shared by
multiple destinations. Missing inputs become black/silent; output and decoder
queues are bounded. See [media path and limits](live-pipeline.md).

The scene editor and encoded output share scene definitions and GPU rendering.
The transient editor camera/guides are separate from the broadcast camera.
Viewport reuse does not duplicate audio. Final GPU-to-CPU readback remains required
before FFmpeg; zero-copy encoding is not implemented.

## Client, server and credentials

The server owns the applied workspace, public policy, domain rules, event history
and typed instructions. Client integrations own provider applications, OAuth,
network protocols, token validation/renewal and private AI credentials. Server
logic guests cannot perform provider I/O or request credentials.

```text
provider -> client WASM/runtime -> normalized event -> server domain -> shared UI
UI/MCP -> server command -> instruction -> authorized client -> provider -> receipt
```

Account panels authorize local sessions. A separate executor registers live
capabilities under saved workspace policy, publishes events, claims instructions
and reports outcomes. Current libraries implement that bridge; an integrated
always-on executor remains unfinished. Registration never grants its own policy.
See [the v2 contract](extension-contract.md), [client executor](client-executor.md)
and [ADR-002](decisions/002-client-execution-and-workspace-capabilities.md).

Committed native macOS credentials use Keychain; browser credentials remain in
memory. Public client extension settings stay in `client_extensions` and are
excluded from server Apply/Start. RTMP stream keys are intentionally part of
workspace configuration and exports; provider/AI secrets are not.

## State and control

The native catalog is a local JSON file; browsers use origin-scoped localStorage.
Apply is separate from local save. A server restart clears the active workspace,
registrations and session history. Reapply the saved client workspace and establish
new live authority; listener settings persist independently in server JSON.
[Persistence](settings-persistence.md) describes merge/conflict behavior.

HTTP UI commands and MCP call the same typed domain handlers. Revision, workspace,
capability and request-intent checks guard mutations. Retried local intents replay
retained results; uncertain external writes are not blindly resent. Cursor reads,
logs, queues, guest fuel/memory and provider payloads have explicit bounds.

The media HTTP API relies on its deployment access boundary and has no user-login
system. MCP uses separate authenticated loopback endpoints with Origin/Host checks.
RTMP source tokens authorize publishers; TLS is required where encryption is needed.

## UI and extension boundaries

Dioxus RSX, CSS, state and GPU widgets are shared across native and browser builds.
Blitz renders the component tree; the browser host is a WASM/WebGPU canvas, not a
second DOM-based application. Dependencies are pinned in Cargo manifests/lockfiles.
See [UI integration](ui.md) for input/rendering qualifications.

Provider URLs, scopes, protocol fields and subscription/retry policy live under
`extensions/`. Hosts render generic manifest/panel declarations and supply bounded
I/O, opaque handles and lifecycle checks. Duplicate installation IDs fail closed.
Adding a provider must not require editing production server/shared UI code.

Product direction and unfinished capabilities are tracked in one
[roadmap](roadmap.md), rather than implementation-session transcripts.
