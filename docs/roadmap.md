# Direction and current boundaries

Acheflow is developing as a studio for content creators. Live production is the
foundation: sources, scenes, audio, outputs and integrations belong to one coherent
workspace. Development should make everyday creation and streaming workflows more
complete while preserving independent local/remote operation and extensibility.

This direction is broader than a transport relay. It is not a claim that a video
editor, recording/export workflow or a complete asset pipeline is already shipped.
New capabilities need focused requirements and verification before they appear as
supported features in user guides.

## Current foundation

- Unified desktop/server executable, owned local server lifecycle and explicit
  native client/server link routing.
- Token-authenticated native-resolution RTMP ingest, scene-scoped stereo mixing,
  GPU composition, 3D viewport editing and independent broadcast destinations.
- Shared native/browser UI, local workspace catalogs, explicit server Apply and
  conflict-aware public control settings.
- Sandboxed Rust/WASM extensions, provider-owned local account setup, generic
  panels, typed event/instruction contracts and authenticated MCP libraries.

## Active completion areas

| Area | Work still needed |
| --- | --- |
| Provider execution | Integrate continuous executor scheduling with the studio, reconnect/registration withdrawal and clear sender/capability availability |
| Provider qualification | Verify configured user-owned applications through consent, renewal, receive/send and actual platform permissions; local OAuth alone does not prove executor behavior |
| Execution durability | Persist/reconcile intents across crashes without automatically repeating uncertain external writes |
| Browser execution | Complete the continuous browser/media executor; account setup is distinct from full provider transport support |
| Interactive integrations | Complete provider poll instruction routing and additional bridge event types; unsupported operations must stay unavailable |
| Installation and links | Coordinate shared-scheme handler/fallback rollout, Developer ID/notarization, signed HTTPS association and target-platform activation tests |
| Platform coverage | Validate Linux desktop packaging/registration and implement Windows native URI activation |
| Media qualification | Measure sustained A/V timing/drift, improve recovery and qualify encoder/device combinations under real workloads |
| UI qualification | Broaden accessibility, IME, focus, clipboard, DPI and hardware performance coverage |

## Expand the studio

Continue improving source organization, scene composition, audio control, preview
clarity and multi-format production. Extend streaming capabilities through the
provider-neutral extension/domain contracts. Broader media creation workflows and
richer scene assets can build on the same architecture, with separate designs for
recording/export, asset import, materials, lights and animation where needed.
These are development directions, not completed features or delivery commitments.

## Current product limits

The server's applied workspace and session history are in memory; there is no
server workspace database. Audio output is one AAC stereo track per destination,
with no local monitoring playback. Final composition copies frames to CPU before
FFmpeg. Output automatic reconnect, HDR/color management, glTF/GLB import, PBR,
lights and authored animation are not implemented as complete product workflows.

A later managed bot/AI offering is optional. It must act as an authorized client
executor and keep provider/AI credentials out of the logic-server process.
Self-operated installations remain a supported architectural goal.
