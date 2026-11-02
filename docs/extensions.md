# WASM extensions

Server guests execute inside wasmi 2.0.0; native/browser UIs render the same
serializable panels. UI WASM and guest WASM are separate artifacts. No Rust/UI/GPU
objects cross the module boundary. The bundled sample stores a counter in WASM
and responds to a panel button. Twitch request/event mapping is guest code;
account authorization and restricted transport are host bindings.

## Installation and limits

Place a guest directory beneath `extensions/`, or `ACHEFLOW_EXTENSIONS_DIR`, with
`manifest.json` and a `.wasm` or `.wat` module. Restart to discover installations,
then start a guest in Extensions. Bundled ids cannot be replaced by external files.
The executable manifest is `acheflow_control::extensions::GuestManifest`, distinct
from the older draft panel metadata:

```json
{"id":"my-extension","name":"My extension","api_version":1,"module":"guest.wasm","capabilities":["panel","scene"],"services":[],"unique_viewers":false}
```

Effects/capabilities: `panel`, `scene`, `chat`, `audience`, `delete`, `delivery`,
`failure`, `http`, `websocket`, `heartbeat`. Unknown capabilities/versions fail.
Network effects additionally need a host account binding; Twitch is the first.
Generic guests receive no account credentials, network/filesystem or media access.
The Twitch binding only attaches credentials to its granted HTTPS origin and
rejects redirects, userinfo URLs and ungranted origins.

Defaults: 8 modules, 4 MiB module, 16 MiB guest memory, 10 million fuel per call,
1 MiB input/output, 64 effects per call, 512 queued events and event-chain depth 4.
Host copy/JSON work also consumes fuel, and template expansion is bounded before
cloning referenced values.
Traps/fuel exhaustion fail the guest visibly. Host action conflicts remain visible
without creating background task queues. Stop cancels work and drops guest state.

## JSON ABI v1

Export linear `memory` and `handle: () -> ()`. Imports in module `acheflow`:

| Import | Signature | Meaning |
|---|---|---|
| `input_len` | `() -> i32` | Current UTF-8 input length |
| `read_input` | `(ptr:i32, capacity:i32) -> i32` | Copy input into checked guest memory |
| `emit_json` | `(ptr:i32, len:i32) -> ()` | Parse/check and emit a raw JSON effect |
| `matches` | `(ptr:i32, len:i32) -> i32` | Compare JSON input pointers to expected values |
| `emit` | `(ptr:i32, len:i32) -> ()` | Emit a template resolved against input |

Templates replace strings beginning `$/` with input-pointer values and support
`{"$map":["$/array","/field"]}`. Conditions support `{"$exists":true}`. Full
programming languages can use raw input/output imports; bundled text guests are
inspectable examples. This versioned executable ABI remains an early contract.

Inputs include `start`, `tick`, `event`, `action`, `gap`, timestamps, request id and
bounded state. Widget actions identify their owning extension, action id and
string value. Events preserve origin/depth; guests do not consume their own output
as new input. Cursor gaps are explicit. Panels support labels, metrics, buttons,
text inputs and selections; no guest HTML/JavaScript executes in the client.

A `scene` effect contains `scene_id`, full `spatial` data, `expected_revision` from
its state snapshot and stable `request_id` from its initiating input. It uses the
same validated scene handler as MCP; busy/stale edits return conflicts.

```sh
python3 extensions/sample/build_guest.py
python3 extensions/twitch/build_guest.py
cargo test -p acheflow-control --locked
cargo test -p acheflow-server control::tests --locked
```

Tests execute actual WASM state/actions, traps, denied capabilities, bounded
memory, invalid imports and fuel exhaustion. See [control workflows](controls.md).
