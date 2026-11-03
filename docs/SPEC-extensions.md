# Spec: extensions

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `extensions` in the
[capability map](../CAPABILITY-MAP.md). This extends the existing
[WASM boundary](extensions.md); it does not claim that the runtime already exists.

Governing revision: [provider-neutral contract v2](extension-contract.md).
Provider OAuth and protocol logic run in Rust WASM guests; the host supplies only
generic transport/secret facilities. Installations are discovered from manifests.

## Objective

Load and execute server WASM extensions with explicit host capabilities. Ship an
actual Twitch guest plus a small deterministic sample guest. The host renders
extension panels consistently in native and browser UI; building the UI to WASM
is separate from executing guest modules.

## Contracts and structure

Extend `acheflow-extension-api` with versioned manifests, guest/host envelopes,
typed events, action schemas and panels. Put server lifecycle, sandbox and host
bindings in new extension modules under `acheflow-server/src`; guest fixtures
under `extensions/`; panel controls in `acheflow-ui/src/extension_panel.rs`.

Manifests declare id, release version, supported host API version, module entry,
requested capabilities and permitted external services. Installation validates
the manifest/module before activation. Host-granted capabilities may be narrower
than requested: event subscriptions, chat read/send, analytics publication, scene
read/update and host-mediated platform HTTP/WebSocket connections are distinct.
Credentials are opaque host handles, scoped to an account and service; requests
are restricted to granted origins/actions. Guests receive no raw tokens or stream
keys, filesystem access, arbitrary network access, UI pointers or media buffers.

The message envelope identifies API version, extension instance, request/event id,
workspace, connection/channel and payload type. Events include provider event id
where available, occurrence/receipt timestamps and a host sequence cursor. Provider
ids are namespaced by connection/channel; replay is observable and deduplicated
before side effects. Concrete ABI encoding/runtime is selected and exercised with
a real guest before implementation expands; Rust pointers are never the ABI.

Lifecycle states: installed, starting, running, stopping, stopped and failed.
Expose last error and granted capabilities. Enable/disable/restart are explicit
commands. Disabling cancels guest work, closes subscriptions and host connections,
and rejects subsequent instance actions. Failure of a guest leaves streaming alive.
The runtime enforces configurable memory, execution/fuel/deadline, message and queue
budgets; defaults and overload behavior must be fixed in the plan.

Panels gain host-rendered text inputs, buttons, selections, chat messages and
connection status as needed by the first integration. UI events return through
typed action ids. Server execution stays on the server; any shipped client logic
must work on native and browser hosts. Arbitrary guest HTML/JavaScript is excluded.

## Acceptance and testing

- Load a real sample WASM guest, invoke an action and observe a panel/event update
  in both UI targets; reject incompatible manifests and malformed envelopes.
- Denied network/scene/chat capabilities cannot be bypassed through host calls.
- Guest traps, deadline exhaustion, queue overload and cancellation produce bounded,
  visible failures; restart creates a new instance without replaying old actions.
- Account credential handles cannot authorize another extension, account or origin.
- Tests exercise the selected runtime, not a native Rust substitute for the guest.

Commands: `cargo test -p acheflow-extension-api --locked`,
`cargo test -p acheflow-server --locked`, and all shared checks in the map after
implementation. Follow the shared Rust/serde style and error contract.

Always validate at guest/host boundaries and keep resource ownership explicit.
Do not freeze the current draft API_VERSION as a stable plugin ABI; reject or
migrate incompatible drafts explicitly. Runtime selection remains a plan decision.
