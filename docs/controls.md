# Interaction, audience, 3D scenes and MCP

Implementation follows the [provider-neutral v2 contract](extension-contract.md).
YouTube/VK Live, model import, lighting and authored animations are later milestones.

## Extensions and interaction

Apply the workspace, enable an extension and fill its manifest-defined settings
inside its card. Provider-defined panel actions handle connection/authorization.
For Twitch, register a public application in the
[Twitch Developer Console](https://dev.twitch.tv/console/apps), enter its Client ID
and use Connect account. Complete the displayed device code. The Rust guest
requests `user:read:chat user:write:chat channel:manage:polls` and operates on the
authorized broadcaster's own channel. No Client Secret is needed. Secret captures
and handles stay in server memory, scoped to the workspace/extension; restart
requires authorization again. OAuth/protocol knowledge is entirely in the guest.

Chat is part of Live / Studio and moves to the shared sidebar during streaming.
Select All channels or one registered destination; available choices are supplied
by enabled extensions. Outgoing messages merge into one history entry with a
result for each target. Receipts suppress duplicate echoes. Timeouts/connection
loss report unknown outcomes and never automatically resend an uncertain message.
Drafts persist when chat moves; narrow layouts place chat below the current page.

Live / Interactive creates actual provider polls on selected capable channels.
Extensions publish initial/periodic/final counts; the host aggregates results and
shows per-target failures. Closing waits for final results. The 60-second UI poll
uses two or more options; each provider declares its own limits. The Twitch guest
supports 2–5 choices, 15–1800 seconds and the service's text limits. There is no
`!vote` fallback. The same page shows normalized donations and direct/registered
interactions. User-created and extension-declared interactions use the common
validated scene action path; users may bind an extension interaction through the
control API. Chat commands retain normalized role checks and cooldowns.

Session history retains 2,000 messages with bounded cursor reads and explicit
gaps. Polls, donations, interactions and channel discovery are session state.
Disabling cancels network work, removes live channels/panels and makes prior
viewer samples unavailable; historical records and public configuration remain.

## Audience

Current audience is the arithmetic sum of distinct platform/channel streams,
without cross-platform deduplication. Duplicate connections must not duplicate a
stream contributor. Confirmed offline contributes zero. Missing/stale data are
unavailable, and partial totals are labelled. Freshness: 90 seconds; history:
1,000 session points. The peak is a simultaneous complete sampled total.

Fetch **unique viewers** separately whenever a provider supports them. Preserve
provider scope, original window and provisional status. Do not sum uniques across
platforms/periods or substitute chatters/concurrent counts. Twitch Get Streams
provides current `viewer_count`; this integration claims no stream-unique endpoint.
Polls, chat and analytics are in-memory session state.

## 3D scenes and viewports

Saved UI catalogs and legacy workspace applications gain a default orthographic
camera and primary viewport, preserving layer ids/order, CSS/crop/effects, active
format selection and scene audio. Standalone legacy model JSON can still be decoded
before that migration step. World space is right-handed, +Y up, camera toward -Z;
default units are legacy canvas pixels. Rotation is Euler XYZ in degrees, and
perspective FOV is in degrees. Canvas resolution and world dimensions are separate.

Live / Scenes has two view modes:

- **Scene editor** uses an independent, transient editor camera. The scene collection
  is on the right, and the selected object's inspector is below the view. Click an
  object or hierarchy row to select; drag a colored world axis to move, rotate or
  scale. `G` / `R` / `S` switch tools. Hold Shift during an axis drag for snapping
  (10 units / 15 degrees / 0.1 scale). Escape restores the drag's starting transform.
- **Camera preview** shows the current active broadcast camera without editor
  guides. The scene currently has one broadcast camera; select Camera in the
  hierarchy or Edit camera to change its position, rotation and lens.

Drag empty space or Alt + drag to orbit; Shift + drag pans, and the wheel zooms.
`F` / Frame focuses the selection; Front / Right / Top choose orthographic working
views. Navigating the editor never changes the broadcast camera or serialized scene.
Edits stay in the workspace draft until **Apply scene** publishes them.

The primary viewport's **Edit viewport content** opens its flat layer canvas and
CSS/crop/effect inspectors. These editors remain mounted while switching objects
or modes, preserving unapplied code drafts. Secondary viewports copy content and
expose source/layer CSS controls; the API accepts their full content contracts.

Shared native/web/headless rendering keeps viewport textures and the depth pass on
the GPU; only the final FFmpeg RGB readback remains. Reused viewports share decoded
inputs and never duplicate audio. Limits: 32 objects, 8 viewports, 32 layers per
viewport, and combined viewport pixels equivalent to two 3840×2160 canvases.
Workspace/revision checks prevent stale 3D overwrites. Workspace version stays 1
with versioned optional `spatial` data; older readers fail on unknown fields.

## MCP and application API

rmcp 3.8.0 implements Streamable HTTP, protocol 2026-07-28 and SDK-supported older
revisions. The dedicated listener binds only loopback. Generate a token with
`openssl rand -hex 32`, then configure the server:

```sh
export ACHEFLOW_MCP_TOKEN='paste-generated-token-here'
export ACHEFLOW_MCP_PORT=21296
cargo run -p acheflow-server --locked
```

Configured tokens must be 32+ characters. Without one MCP stays disabled. Configure
an MCP client with `http://127.0.0.1:21296/mcp` and an `Authorization: Bearer <token>`
header. Optional distinct `ACHEFLOW_MCP_READ_TOKEN` enables `/mcp/read`, which only
exposes reads. Origin/Host validation rejects untrusted browser requests. Public
hosting and stdio are outside this release.

- `acheflow_state`: scene, chat, polls, commands, extension status/panels and audience;
  optional bounded `after`/`limit` support cursor reads.
- `acheflow_command`: chat/poll/interaction, scene and generic extension/panel actions.

Commands require workspace id, expected revision, action and a request id shaped
`unix_milliseconds:unique_suffix`. Reuse the id/arguments for retries of one intent.
Ids are valid ten minutes; the host retains 1,024 results, reserves before effects,
rejects in-progress duplicates, changed payloads and expired results. After a server
restart, establish new intents rather than replaying external sends.

The native/web client uses `GET /api/control/state` and
`POST /api/control/commands`, under the existing application HTTP/proxy trust
boundary. MCP reuses those service handlers and exports no OAuth secrets, stream
keys, shell or raw GPU access.

## Verification

Tests execute Twitch WASM with provider-shaped fixtures, real sample panel actions,
bounded interaction/analytics and an actual MCP SDK client over loopback. Metal
compares default viewport pixels to legacy output and checks depth. RTMP coverage
runs a WASM-mapped chat command through a 3D object edit and real encoded output.
Live Twitch OAuth/EventSub/send checks additionally require a registered Client ID
and user account authorization; fixture tests do not prove that live check.
Native pointer tests additionally cover 3D picking, axis dragging outside the view,
Escape cancellation and camera-preview isolation. Run all checks in the [capability map](../CAPABILITY-MAP.md).

Official sources: [Twitch device authorization](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/#device-code-grant-flow),
[chat authorization](https://dev.twitch.tv/docs/chat/authenticating/),
[EventSub lifecycle](https://dev.twitch.tv/docs/eventsub/handling-websocket-events/),
[Get Streams](https://dev.twitch.tv/docs/api/reference/#get-streams),
[native polls](https://dev.twitch.tv/docs/api/polls),
[MCP transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).
