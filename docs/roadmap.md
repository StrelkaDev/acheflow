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
| Provider execution | Qualify native Studio reconnect, receipt reconciliation and live provider availability; continuous browser execution remains unfinished |
| Provider qualification | Verify configured user-owned applications through consent, renewal, receive/send and actual platform permissions; local OAuth alone does not prove executor behavior |
| Execution durability | Persist/reconcile intents across crashes without automatically repeating uncertain external writes |
| Browser execution | Complete the continuous browser/media executor; account setup is distinct from full provider transport support |
| Interactive integrations | Complete provider poll instruction routing and additional bridge event types; unsupported operations must stay unavailable |
| Installation and links | Coordinate shared-scheme handler/fallback rollout, Developer ID/notarization, signed HTTPS association and target-platform activation tests |
| Platform coverage | Validate Linux desktop packaging/registration and implement Windows native URI activation |
| Media qualification | Measure sustained A/V timing/drift, improve recovery and qualify encoder/device combinations under real workloads |
| UI qualification | Broaden accessibility, IME, focus, clipboard, DPI and hardware performance coverage |

## Existing-logic backlog

These tasks improve existing behavior. Code-review findings need a regression
reproduction before implementation; timing risks need measurements before choosing
a correction. Unchecked items are open. P1 covers state consistency and lost edits;
P2 covers recovery and completion of existing mechanisms; P3 covers maintainability.
New bugs found during everyday use can change this order.

Start with LOGIC-04 and LOGIC-03, then LOGIC-01 and LOGIC-02. Keep each task a
separate reviewable change. For every implementation, run the affected regression
tests and applicable [development checks](development.md#standard-checks), update
the owning guide, and only then mark the task complete. Keep provider-specific
behavior under `extensions/` and follow the [extension contract](extension-contract.md).

### LOGIC-01: Atomic extension configuration — P1

Settings are currently written before the guest successfully starts. A rejected
operation can leave changed workspace settings without a matching control revision.

Owner: [control](../crates/acheflow-server/src/control.rs),
[workspace extensions](../crates/acheflow-server/src/workspace_extensions.rs),
[guest lifecycle](../crates/acheflow-server/src/extension_host.rs). Dependencies: none.

- [ ] Validate the installation and requested configuration before mutating settings.
- [ ] Define and enforce one consistent failure outcome for settings, runtime status
  and revisions; never leave a failed startup reported as a running instance.
- [ ] Cover unknown installations, invalid configuration, guest-load failure and
  successful enable/disable with regression tests using real WASM guests.

### LOGIC-02: Protect all workspace edits from stale Apply — P1

Revision conflicts currently protect extension settings, capability grants and
spatial scenes, but do not cover every media configuration change.

Owner: [workspace application](../crates/acheflow-server/src/live.rs),
[client synchronization](../crates/acheflow-ui/src/control_client.rs). Dependencies: none.

- [ ] Reproduce conflicting edits from two clients for layers, audio routing and
  output settings, then define the expected-revision and explicit resolution rules.
- [ ] Reject or explicitly reconcile stale Apply without silently discarding another
  client's edits; preserve unrelated local drafts and live-edit restrictions.
- [ ] Test simultaneous edits, accepted conflict resolution and server restart;
  verify the conflict is understandable in the UI.

### LOGIC-03: Ignore stale control responses — P1

A background snapshot request can finish after a newer command and replace its
acknowledged state. Requests also need bounded lifetime and workspace ownership.

Owner: [control client](../crates/acheflow-ui/src/control_client.rs). Dependencies: none.

- [ ] Add request-generation or equivalent ordering checks so a delayed snapshot
  cannot undo a newer command response or lower the acknowledged edit revision.
- [ ] Bound requests and discard responses from a previous workspace/server;
  recover busy state after timeout and allow a restarted server to establish new state.
- [ ] Test delayed GET/command responses, workspace switching and timeout recovery;
  manually verify native/browser controls remain usable.

### LOGIC-04: Reclaim inactive executor registrations — P2

Expired registrations still count toward the client limit. The limit can be exhausted
with no live executors; this behavior has a focused reproduction.

Owner: [execution registry](../crates/acheflow-control/src/client_execution.rs),
[registry tests](../crates/acheflow-control/tests/client_execution.rs). Dependencies: none.

- [ ] Reclaim inactive registration capacity using a bounded retention policy.
- [ ] Preserve generation/replay barriers and cancellation or Unknown outcomes for
  instructions belonging to withdrawn authority; apply bounded retention to policies too.
- [ ] Test more than 64 successive expired clients, active-capacity limits, stale
  generations, retained instructions and isolation between workspaces.

### LOGIC-05: Recover from bounded audio lag — P2

An audio receiver lag currently fails the entire destination, using the same path
as a closed audio channel. Recovery must preserve the output timeline.

Owner: [output worker](../crates/acheflow-server/src/output.rs),
[audio mixer](../crates/acheflow-server/src/audio_mixer.rs). Dependencies: none.

- [ ] Reproduce lag with a deliberately slow consumer and distinguish Lagged,
  channel closure, cancellation and socket-write failure.
- [ ] Define a bounded resynchronization policy with an explicit unrecoverable
  threshold; do not accumulate latency or silently corrupt A/V timing.
- [ ] Verify short stalls recover, persistent failures remain visible, and another
  destination keeps delivering media during the affected output's recovery.

### LOGIC-06: Reconnect failed destinations — P2

Independent manual restart exists; automatic reconnect remains unfinished.

Owner: [destination lifecycle](../crates/acheflow-server/src/compositor.rs),
[output worker](../crates/acheflow-server/src/output.rs). Depends on LOGIC-05.

- [ ] Add bounded retry/backoff for recoverable failures with visible attempt status
  and an explicit terminal outcome for permanent errors or exhausted retries.
- [ ] Ensure Stop, deactivation and workspace changes cancel pending retries and
  retire old workers before replacement; manual restart must not create duplicates.
- [ ] Test connection loss/recovery with real FFmpeg output and two destinations,
  including cancellation during backoff.

### LOGIC-07: Measure sustained A/V timing — P2, investigation

Video uses the latest decoded image and receipt time, while audio has its own PTS
mapping. Sustained drift under load has not been established by this review.

Owner: [ingest](../crates/acheflow-server/src/ingest.rs),
[audio playout](../crates/acheflow-server/src/audio_mixer.rs),
[media tests](../crates/acheflow-server/src/pipeline_tests.rs). Dependencies: none.

- [ ] Build a repeatable real-media check with synchronized visual/audio markers;
  measure offset, drift and delivery continuity at the receiving output.
- [ ] Exercise a sustained baseline, CPU/GPU load, source reconnect, timestamp
  discontinuities and independent output restart; define duration and tolerances.
- [ ] Identify whether a timing correction is needed and the owning clock/queue;
  keep raw recordings and environment-specific results outside tracked docs.

### LOGIC-08: Correct measured timing failures — P2, conditional

Choose the correction from LOGIC-07 evidence rather than assuming a particular
clock redesign. If the agreed tolerances pass, close this item with that rationale.

Owner: the ingest, mixer, compositor or output path identified by LOGIC-07.
Depends on LOGIC-07.

- [ ] Specify the failing timeline transition and the expected timestamp, frame
  drop/duplication or silence policy before implementing a correction.
- [ ] Keep queues and latency bounded across source/output reconnects and load;
  preserve relative timing between a source's audio tracks.
- [ ] Pass the same baseline and stress checks used to reproduce the failure,
  including both output orientations.

### LOGIC-09: Own executor registration in Studio — P2

Native Studio now owns enabled local provider runtimes and their registration
lifecycle. Further qualification remains distinct from account authorization.

Owner: [executor bridge](../crates/acheflow-client/src/bridge.rs),
[account lifecycle](../crates/acheflow-ui/src/client_authorization.rs). Depends on LOGIC-04.

- [x] Tie native executor registration, heartbeat and bounded reconnect to the active
  workspace/server and authorized local account session.
- [ ] Withdraw authority on account removal, disable, workspace switch and shutdown;
  make advertised capabilities and availability reflect current live authority.
- [ ] Verify two independent real WASM providers, expiry/generation renewal and
  cancellation; preserve provider-neutral host/UI code.

### LOGIC-10: Run provider events and instructions continuously — P2

Native receive, ticks, publication and instruction claims are scheduled by Studio.
Keep broader transport-failure and receipt-reconciliation qualification open;
account setup alone must not be reported as a connected executor.

Owner: [provider runtime](../crates/acheflow-client/src/runtime.rs),
[executor bridge](../crates/acheflow-client/src/bridge.rs). Depends on LOGIC-09 and LOGIC-11.

- [ ] Schedule receive, normalized event publication and instruction claims without
  starving heartbeat or cancellation; bound queues and concurrent provider I/O.
- [ ] After reconnect or authority changes, publish only from the current enabled
  instance and execute only its authorized claims.
- [ ] Verify receive/send through two independent real WASM providers, loss of
  transport and workspace switching; keep unsupported browser execution explicit.

### LOGIC-11: Retry execution receipts without repeating provider writes — P2

A provider action can finish before Report fails. Reconciliation must retry the
receipt, preserving the existing prohibition on blind external-write retries.

Owner: [executor bridge](../crates/acheflow-client/src/bridge.rs),
[local receipt ledger](../crates/acheflow-client/src/runtime.rs). Dependencies: none.

- [ ] Retain a bounded pending-report queue and retry identical receipts while
  their lease/session remains valid; expose terminal reconciliation failures.
- [ ] Preserve Unknown outcomes and idempotent reporting without executing the
  provider action again or extending revoked authority.
- [ ] Test lost Report requests/responses, reconnect, duplicate receipts, expiry
  and shutdown; verify the external side effect happens at most once per claim.

### LOGIC-12: Restore execution reconciliation after crashes — P2

Execution and receipt state are currently in memory. Durable recovery needs an
explicit retention and server-session boundary.

Owner: [local receipt ledger](../crates/acheflow-client/src/runtime.rs),
[execution registry](../crates/acheflow-control/src/client_execution.rs). Depends on LOGIC-11.

- [ ] Define and implement a bounded durable intent/receipt journal with no provider
  credentials, covering reservation before I/O and final outcome before reporting.
- [ ] Reconcile restored entries with current server authority; make stale leases,
  server restart and uncertain external writes explicit rather than resending them.
- [ ] Test crashes before the write, after a possible write and after a lost report;
  also test corrupted journal data, retention and isolated workspaces.

### LOGIC-13: Split workspace validation by responsibility — P3

Owner: [workspace validation](../crates/acheflow-media-api/src/workspace.rs),
[runtime validation](../crates/acheflow-server/src/live.rs). Depends on LOGIC-02.

- [ ] Separate source, scene, output and cross-reference validation while retaining
  the distinction between editable drafts, Apply and executable Start configuration.
- [ ] Reuse canonical rules across client/server without relaxing limits or changing
  supported protocols, disabled-output behavior or migration semantics.
- [ ] Preserve accepted/rejected cases with regression tests and compare Repowise
  health before/after; helpers must reduce branching, not only move it.

### LOGIC-14: Separate control command handling — P3

Owner: [ControlHost](../crates/acheflow-server/src/control.rs).
Depends on LOGIC-01 and LOGIC-02.

- [ ] Extract handlers by domain responsibility while retaining one canonical
  reservation, authorization, locking and revision-completion boundary.
- [ ] Preserve request replay, partial-failure outcomes, workspace ownership and
  live-instance checks across HTTP, MCP and extension-origin commands.
- [ ] Run affected control/real-WASM regressions and compare Repowise health
  before/after; avoid introducing a parallel command-dispatch implementation.

## Bugs found during use

Add each new report here as `BUG-001`, `BUG-002`, and so on, or link it to the
matching LOGIC task when it is the same problem. Reports do not need a suspected
cause. Record:

- Short title and impact: blocks work, loses edits/media, or causes inconvenience.
- App version, native/browser mode, OS, local/remote server, and relevant media setup.
- Minimal steps, expected behavior, actual behavior and reproduction frequency.
- Whether it follows Apply/Start/Stop, workspace switching, reconnect or restart;
  include a screenshot or sanitized log excerpt when useful.

Use placeholders for identities and configuration. Keep original logs, recordings,
tokens, account/workspace identifiers and machine paths outside tracked docs.
User reports are awaiting reproduction until verified; give data loss and blocked
core workflows priority over cleanup work.

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
