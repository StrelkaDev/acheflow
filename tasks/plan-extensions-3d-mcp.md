# Extensions / interaction / 3D / MCP implementation

The user approved implementation on 2026-11-02. Specifications are indexed by
`CAPABILITY-MAP.md`. Preserve the pre-existing uncommitted media/UI work. Do not
commit or merge without a separate request.

1. Add shared platform events and a separately testable control service. Prove
   audience summation, optional uniques, freshness, deduplication and poll voting.
2. Execute actual WASM guests with wasmi, metered instructions and bounded memory.
   Use a small JSON message ABI, with host-owned I/O and opaque credentials.
   Prove traps, denied capabilities, stop/restart and real guest execution.
3. Run Twitch guest normalization and request mapping over host-mediated HTTPS and
   EventSub WebSockets; implement account authorization, reconnect and status.
4. Add scene camera/objects/viewports and legacy-compatible loading. Render 2D
   textures into a shared GPU 3D pass; wire editor controls and encoded output.
5. Expose shared service commands through authenticated loopback MCP, using rmcp;
   verify the supported protocol revision and a real client round trip.
6. Integrate native/web chat, analytics, polls and extension controls. Run project
   checks, real GPU/media checks, browser verification and final change review.

Initial limits: 8 loaded extensions, 16 MiB guest linear memory, 4 MiB module,
10 million fuel per invocation, 1 MiB input/output, 512 pending events, 2,000 chat
messages, 1,000 analytics points, 90-second viewer freshness and 30-second polling.
Provider retry/rate limits override polling cadence. Session analytics are in-memory.
Local commands retain 1,024 request results; expired request ids require a new
intent rather than silently replaying an external effect.

Scene convention: right-handed, +Y up, camera looks toward -Z, world units equal
legacy canvas pixels for the default orthographic view. Euler XYZ rotations in
degrees; perspective FOV in degrees. At most 32 objects and 8 viewports per scene.
Legacy `layers` remain the primary viewport's serialized content for compatibility;
additional viewport content is explicit. New scene data uses its own schema version
so older loaders reject unknown spatial fields instead of discarding them.

Use existing required commands from the capability map. Focused tests run at each
slice; full native/WASM, Clippy, Trunk and GPU/media checks run after integration.

Editor revision requested by the user: separate active-camera preview from a
transient orbit camera; hierarchy beside the view, inspector below it; shared
projection/picking math, world-axis move/rotate/scale, snapping, framing and Escape.
Keep viewport code editors mounted when hidden. Later lightweight glTF/GLB assets
need PBR, lights and animation; those are separate renderer/asset capabilities.
