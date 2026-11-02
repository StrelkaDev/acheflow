# Capability map: extensions, interaction, 3D scenes and MCP

Status: scope/specifications approved in chat on 2026-11-02; implementation is in
the working tree. See [operational details and qualifications](docs/controls.md).

## Outcome and scope

Acheflow hosts platform integrations as extensions. External MCP clients control
Acheflow; Acheflow is not an MCP client for platform integrations in this milestone.
Twitch is the first working integration. YouTube and VK Live follow the same host
contracts in later milestones.

A render scene contains a camera, viewport planes and basic 3D objects. The existing
2D layer composition, including CSS, crop and WGSL effects, becomes viewport
content. First-release primitives are a plane and a cube. Model import, complex
materials, lighting and authored animation are subsequent work.

| Module id | Responsibility | Depends on | Specification |
|---|---|---|---|
| `extensions` | Versioned contracts, server WASM execution, host capabilities, panels and lifecycle | Existing extension API | [SPEC-extensions](docs/SPEC-extensions.md) |
| `interaction` | Normalized chat, outgoing messages, commands and application polls | Extension contracts/event delivery | [SPEC-interaction](docs/SPEC-interaction.md) |
| `analytics` | Current viewers, sum, session history and optional platform uniques | Extension contracts/event delivery | [SPEC-analytics](docs/SPEC-analytics.md) |
| `scene-3d` | Camera, viewport textures, primitives, transforms and workspace migration | Existing media API and GPU composition | [SPEC-scene-3d](docs/SPEC-scene-3d.md) |
| `twitch` | Account/channel connection, incoming/outgoing chat and viewer samples | Extensions, interaction and analytics contracts | [SPEC-twitch](docs/SPEC-twitch.md) |
| `mcp` | External read/control adapter over application services | Extensions, interaction, analytics and scene services | [SPEC-mcp](docs/SPEC-mcp.md) |

Build order: extension contracts/runtime → interaction and analytics with Twitch →
3D scenes and workspace migration → MCP → end-to-end chat-triggered scene action.
The 3D module is independent of platform APIs. Service interfaces belong to their
provider specifications; consumers do not redefine them. Extension host services
are injected into the runtime, avoiding a dependency cycle with domain modules.

## Shared implementation conventions

Keep shared serializable types in `acheflow-extension-api` (guest/host messages)
and `acheflow-media-api` (workspace and application state). Server services and
transport handlers live in separate modules under `acheflow-server/src`; GPU work
lives in `acheflow-composition/src`; native/web components live in
`acheflow-ui/src`. Guest code and fixtures belong under a new `extensions/` tree.
These are proposed locations, not existing implementations.

Use Rust 2024, typed serde contracts, snake_case JSON fields and tagged enums,
consistent with the existing APIs. For example:

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewerMetric {
    Concurrent { value: u64 },
    Unique { value: u64, period_start_ms: u64, period_end_ms: u64 },
}
```

This is illustrative draft syntax. Published contracts must also carry the
connection, channel, stream/session, timestamps, availability and provider scope
defined in the module specs. Incompatible message versions fail explicitly.

UI, extensions and MCP call the same application command handlers. Commands carry
workspace identity, actor/capabilities and a request id. Mutations check an expected
revision when updating existing state. Domain failures use stable codes:
`invalid_argument`, `not_found`, `forbidden`, `conflict`, `unsupported`,
`unavailable`, `rate_limited`, `cancelled` and `outcome_unknown`.

Bound all histories, message sizes, queues and event batches; expose cursor gaps
instead of silently losing events. Each module specifies initial defaults before
implementation. Expired cursors return a gap and a fresh snapshot cursor. A timeout
after an external send is an unknown outcome, not permission to resend blindly.

## Verification and boundaries

Tests stay beside the relevant Rust modules; server integration coverage extends
the existing pipeline test arrangement. Guest fixtures prove actual WASM execution,
not only manifest parsing. Deterministic adapter fixtures run without accounts;
account-dependent checks are explicitly reported separately.

Required implementation checks, from the current README:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p acheflow-ui --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
cargo test -p acheflow-server --locked -- --ignored --nocapture
cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture
trunk build
```

Always preserve existing streaming, native/web UI, audio routing and saved
workspaces; validate external messages and permissions at their entry boundary.
Runtime/dependency selection and concrete resource limits must be justified in
the implementation plan. A breaking workspace/ABI change requires an explicit
migration contract, not silent reinterpretation. Never expose stream keys or
OAuth tokens through extension panels, chat, analytics or MCP.

First-release acceptance: connect Twitch, read/send chat and see current viewers;
run an application poll; trigger a bounded object transform from a permitted chat
command and from MCP; observe the result in both preview and encoded output.
Old workspaces produce equivalent default viewport imagery and audio.

Runtime/SDK decisions: wasmi 2.0.0, rmcp 3.8.0 and MCP 2026-07-28 with SDK backward
compatibility. See the implementation plan in `tasks/plan-extensions-3d-mcp.md`.
Live account verification additionally requires Twitch application registration and
user authorization; fixture tests do not substitute for that check.
