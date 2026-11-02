# Interaction, audience, 3D scenes and MCP

Implementation follows the [capability map](../CAPABILITY-MAP.md). Twitch is first;
YouTube/VK Live, native platform polls, model import, lighting and authored
animations are subsequent milestones.

## Twitch and interaction

Apply the workspace, then use Extensions / Connect Twitch with a **public Twitch
application Client ID**. Register the application in the
[Twitch Developer Console](https://dev.twitch.tv/console/apps). Complete the
shown device code on Twitch. Requested scopes: `user:read:chat user:write:chat`.
The installed client operates on the authorized broadcaster's own channel.
OAuth/refresh tokens stay in server memory, outside guests, workspaces and MCP;
server restart requires reauthorization. No Client Secret is needed.

EventSub receives chat; Twitch API sends messages and samples current viewers
every 30 seconds, subject to provider retry limits. The binding handles reconnect
handoff, keepalive, token refresh and event deduplication. Sends target one selected
channel. A timeout reports an unknown outcome and never causes automatic resend.

Chat is part of Live / Studio. While a live session is running, it moves into a
shared right sidebar that stays visible across all application and Live tabs.
The message draft and outgoing channel persist when it moves. On narrow screens,
the shared chat sits below the current page. Polls and scene commands have their
own Live / Interactive tab, with a separate channel selection.

Application polls accept `!vote <number>`; the first valid vote per namespaced
provider user counts, including across targeted channels. UI duration is 60 seconds;
API duration is 10–86400 seconds. Announcement failure is visible on the poll.
Commands bind a selected 3D object to a finite rotation, role policy and cooldown;
the initial UI offers 45-degree Y rotation and a five-second cooldown. Chat never
executes a shell or arbitrary MCP tool. Session history retains 2,000 messages;
initial reads show a recent page, cursor reads advance with explicit gaps.

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
- `acheflow_command`: chat/poll/command, scene, extension/panel and authorization actions.

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
[MCP transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).
