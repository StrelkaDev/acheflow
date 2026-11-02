# Spec: scene-3d

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `scene-3d` in the
[capability map](../CAPABILITY-MAP.md).

## Objective

Promote the current layer scene into viewport content inside a 3D render scene.
Provide a camera, multiple viewport planes, basic colored planes/cubes and editable
3D transforms. Preview and broadcast use the same scene definition and renderer.

## Contracts and structure

Versioned workspace/scene types and migration live in `acheflow-media-api`;
GPU composition in `acheflow-composition`; active-scene commands in server services;
scene hierarchy, camera/object controls and viewport layer editing in the UI.

A scene has a stable id, orientation/output relationship, active camera, objects
and its existing scene-scoped audio routing. Objects have stable ids, names,
visibility, position, rotation and scale. First-release types are a viewport plane,
a colored plane and a colored cube. A viewport owns its 2D canvas resolution,
source layers, crop, CSS and WGSL effects, separately from its world dimensions and
transform. Several viewports may reference the same source without extra decoders.

Use a documented right-handed world with +Y up, local transforms and explicit
units. Specify camera view/projection, rotation representation/order and clip-space
conversion in the implementation plan and lock them with tests. Support orthographic
and perspective cameras with validated clipping planes, aspect and projection
parameters. Reject nonfinite coordinates, invalid dimensions/scales, missing source
or camera references and excessive scene budgets before applying changes.

Expose scene snapshot, create/delete/select scene, object create/update/delete,
camera update and viewport-content update commands. Each mutation validates ids,
workspace/capabilities and expected revision, then atomically publishes a complete
scene snapshot. Concurrent MCP/editor/extension edits conflict visibly rather than
overwriting one another. Chat action bindings invoke these same commands.

Render viewport 2D compositions into sampleable GPU textures, then draw the camera
view with depth testing. Keep GPU device/queue and resource lifetime coordinated
between 2D and 3D passes; no intermediate CPU readback/upload. Preserve the current
final readback boundary for FFmpeg. Verify texture orientation, alpha/color handling,
occlusion, clipping and resize behavior on native headless and browser targets.

Migrate each legacy scene into a scene with one viewport and a default orthographic
camera. Transfer existing layer ids, ordering, crop, CSS and effects unchanged.
The default view fills the former canvas and reproduces its image. Retain active
horizontal/vertical scene selection and scene audio mix; multiple viewport copies
must not duplicate audio. Saving uses a versioned format; unsupported future
versions fail clearly. Never destructively overwrite an unmigrated workspace on
failed load/validation. Existing dirty editor drafts retain their identities.

## Editor interaction

The editor camera is transient UI state, distinct from the active broadcast camera.
Provide separate Scene editor and Camera preview modes, a scene hierarchy beside the
working view, and an inspector below it. Shared projection/transform math must keep
visible axis handles, picking and manipulation aligned on native and high-DPI web
surfaces. Support axis move/rotation/scale, orbit/pan/zoom, selection framing, standard
orthographic views, snapping and Escape cancellation. Working-camera navigation and
all editor guides stay out of broadcast output. Preserve viewport code drafts across
mode and object changes.

Future lightweight GLB/glTF assets need PBR materials, lights and animation support.
Keep object identity/transforms and view navigation independent from primitive mesh
rendering so those capabilities can extend the scene without replacing the editor.
Importing assets, lighting and PBR are not part of this iteration.

## Acceptance and testing

- Legacy workspace fixtures migrate, save/reload and render equivalent images and
  audio; layer identities and horizontal/vertical selection survive.
- Multiple independently transformed video viewports and a cube appear correctly
  in preview and real encoded output, with depth and perspective verified.
- Camera/object controls and viewport layer editing work in native and browser UI;
  selecting another object preserves dirty viewport editor drafts.
- GPU tests check default-view equivalence, texture corners/orientation, occlusion,
  alpha, resize and shared-source behavior; invalid scenes and stale revisions fail.
- An authorized command changes a transform; unauthorized or malformed commands
  cannot alter the active scene.

Commands: `cargo test -p acheflow-media-api --locked`,
`cargo test -p acheflow-composition --locked`,
`cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture`,
plus server pipeline tests and every shared implementation check. Follow shared
Rust/serde style and keep renderer-specific state out of serialized contracts.

Always migrate old composition behavior explicitly and preserve audio routing.
GLTF/GLB import, lighting, complex materials, authored animations, physics and
arbitrary guest GPU/media access are later capabilities.
