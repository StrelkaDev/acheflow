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
| Provider execution | Keep shared chat collection on the server; migrate personal sending to device-local extension execution (SHARED-09–13) |
| Provider qualification | Verify configured user-owned applications through consent, renewal, receive/send and actual platform permissions; local OAuth alone does not prove executor behavior |
| Execution durability | Persist/reconcile intents across crashes without automatically repeating uncertain external writes |
| Browser execution | Qualify direct personal actions under browser OAuth/CORS/transport restrictions; shared chat is read from the server, without a browser collector |
| Interactive integrations | Complete provider poll instruction routing and additional bridge event types; unsupported operations must stay unavailable |
| Installation and links | Coordinate shared-scheme handler/fallback rollout, Developer ID/notarization, signed HTTPS association and target-platform activation tests |
| Platform coverage | Validate Linux desktop packaging/registration and implement Windows native URI activation |
| Media qualification | Measure sustained A/V timing/drift, improve recovery and qualify encoder/device combinations under real workloads |
| UI qualification | Broaden accessibility, IME, focus, clipboard, DPI and hardware performance coverage |

### Studio editing and preview

1. **Qualify encoded output preview.** Studio can compare framing and local
   compression for each configured destination. Verify the supported codecs
   and hardware encoders under load, and measure how concurrent previews affect
   broadcast frame timing and reconnects.
2. **Extend 30/60 FPS qualification to production scenes.** The
   [mixed RTMP/SRT probe](development.md#mixed-output-preview-load-at-30-and-60-fps)
   covers both orientations, raw and encoded previews, and an independent
   output restart at 720p. Extend to higher resolutions and scenes with
   effects, and measure browser-visible freshness and sustained A/V timing.
3. **Make Studio the editing workspace.** Keep the draft scene, on-air scene,
   mixer and output state in view. Add per-source preview, independent audio
   monitoring and a focused inspector. Verify keyboard and phone workflows.
4. **Reduce monitoring overhead.** Preserve peak audio values between updates
   and stream fresh meter and load samples separately from workspace state.
   Verify clipping visibility and reconnection without duplicating graph points.
5. **Profile layer and neural effects.** Measure GPU work by render pass and
   time model inference and missed frames for effects that dispatch to a
   neural accelerator.
6. **Replace whole-workspace Apply in production controls.** Mixer edits,
   scene content, program switches, source cards and saved output profiles use
   scoped updates. Extend scoped saves to format settings and destination
   additions, removals and enablement.

## Shared server workspace

Target: [ADR-003](decisions/003-trusted-server-workspace.md), refined by
[ADR-004](decisions/004-extension-client-server-ownership.md). One server owns one
active workspace; connected clients share state with independently granted rights.
No cloud account is required.

### Agreed extension model

- Extensions can provide client functionality, server functionality, or both.
  Packages are installed on the host that executes each part; the manifest/SDK
  declares the available parts and their capabilities.
- The server part has its own platform authorization and is the only collector
  for shared chat. Clients read the server feed within their Acheflow grants.
- Each device authorizes its client part separately. Personal sending, and later
  moderation, run directly against the platform using that device's account.
  Linking devices to one Acheflow user does not synchronize platform credentials.
- Settings use **Client / Server** tabs. Server installation, configuration,
  authorization and execution are enforced by server-side permissions; viewing
  settings never exposes credentials. Client settings affect only that device.
- Personal credentials stay on their device. Each host uses one credential
  container per workspace, with isolated extension/application/account slots.
  Server credentials belong to the server's separate host-owned vault.
- Acheflow grants and platform rights both constrain supported client actions.
  The app can enforce its own flow, but cannot prevent use of a locally held
  platform token outside Acheflow. Never silently switch to a server account.

### Current implementation and migration order

Keep shared workspace persistence/synchronization, user/device sessions, role
checks, the server WASM collector and its lifecycle/receipt safeguards. Local
vaults already group extension slots. These are foundations, not proof that the
new ownership model is complete.

Repository packages declare independent client/server parts. Authorized ZIP
installation, server-only common-feed collection, direct device sends, independent
server authorization and Client / Server settings are implemented. Old personal
upload/send APIs are retired (410); owner-scoped metadata and explicit cleanup
remain for safe reauthorization. Vaults use stable server identity and cached,
locked workspace containers. Browser client packages remain bundled; native
client package activation requires restart.

The execution order below is authoritative; task numbers identify scope, not
sequence. **SHARED-07 implementation and automated acceptance are complete. SHARED-08 is
still the final qualification gate.** Code for SHARED-10–13 is implemented; the
physical-phone, live-provider and signed-build Keychain checks below remain explicit
release checks and must not be mistaken for completed end-to-end qualification.

| Order | Task | Completion required before moving on |
| --- | --- | --- |
| 1 | [SHARED-09](#shared-09-declare-client-and-server-extension-parts--complete) | Define extension parts and compatibility before changing loaders/installers |
| 2 | [SHARED-07](#shared-07-install-extension-packages-through-authorized-clients) | Install the declared parts on the selected host with package validation and permission checks |
| 3 | [SHARED-10](#shared-10-collect-shared-chat-only-on-the-server) | Keep the server as the sole shared-chat collector and preserve client reads |
| 4 | [SHARED-11](#shared-11-execute-personal-actions-on-the-originating-device) | Move personal sending to the device; provide explicit reauthorization before retiring server personal execution |
| 5 | [SHARED-12](#shared-12-finish-vault-ownership-and-keychain-access-behavior) | Finish host-owned workspace containers and reduce repeated Keychain access |
| 6 | [SHARED-13](#shared-13-show-host-specific-settings-and-authorization) | Expose Client / Server settings, independent authorization and permission enforcement |
| 7 | [SHARED-08](#shared-08-qualify-the-shared-studio-workflow) | Qualify the complete desktop/phone workflow and package lifecycle after all preceding tasks |

Each task remains open until its own acceptance checks pass. A later task's design
or partial implementation does not complete SHARED-07 installation or SHARED-08
end-to-end qualification.

Built-in moderation remains future work. Preserve the action boundary and capability
checks now; do not implement moderation or server-account delegation as a prerequisite
for restoring chat and personal sending. Media and unrelated studio tasks remain
in scope of the existing backlog; continuous client chat collection is superseded.

### SHARED-01: Consolidate workspace credential storage

Dependencies: none. Owner: `acheflow-client` vault and local authorization guide.

- [x] Group extension/application slots in a macOS server/workspace record;
  removing one account preserves its neighbors. The local and server namespaces
  are currently separate and device access sessions use another record. Full
  target ownership and repeated-access qualification remain SHARED-12.
- [x] Migrate old records only after the replacement is saved. Failed reads,
  writes or cleanup must retain recoverable data and never fall back to plaintext.
- [x] Serialize concurrent updates, including separate native processes; test
  isolation, failure recovery, bounds and a real isolated Keychain roundtrip.

### SHARED-02: Authenticate and authorize connected devices

Depends on ADR-003. Owner: server access middleware, shared API and client access UI.

- [x] Bootstrap the local administrator; issue expiring, one-use invitations with
  bounded attempts, revocable device sessions and no implicit proxy/loopback trust.
- [x] Enforce separate view/chat/scene/media/extension/install/admin capabilities
  on every existing HTTP, preview, control and MCP entry point. Reject privilege
  escalation and redact sensitive fields before returning viewer state.
- [x] Test direct unauthorized calls, two distinct clients, invitation replay,
  session revocation, cross-origin requests and HTTPS-proxy operation.

### SHARED-03: Persist the server workspace

Depends on SHARED-02. Owner: server storage/live API, workspace models and local import.

- [x] Store and retrieve the active configuration with stable identity and revision;
  restore after restart without automatically starting outputs or provider writes.
- [x] Explicitly import local workspaces without deleting the source catalog or
  transferring local accounts. Client connection alone cannot switch workspaces.
- [x] Test interrupted saves, restart, validation failures and permission-filtered
  snapshots. Cover every stale configuration write, including LOGIC-02.

### SHARED-04: Synchronize all connected editors

Depends on SHARED-03. Owner: workspace, live/control clients and authenticated workspace polling API.

- [x] Desktop and browser attach to the same active workspace and receive ordered
  updates, including media, scenes and extension settings. View selection stays local.
- [x] Preserve unsent drafts; resolve stale writes explicitly. Discard responses
  from old sessions/workspaces and recover from server restart (including LOGIC-03).
- [x] Verify two editors, read-only views, concurrent edits, disconnect/reconnect
  and clean cancellation, with meaningful conflict controls in native/browser UI.

### SHARED-05: Execute integrations once on the trusted server

Depends on SHARED-02 and SHARED-03. Owner: generic executor host and extension contract.

- [x] Host provider WASM with bounded transport and isolated secrets on the server;
  preserve provider-owned auth/URLs/protocol logic and separate local device features.
- [x] Fence executor ownership per binding and collection per provider/channel;
  opening another client or authorizing a personal action account cannot create a
  second chat subscriber. Disable, replacement and workspace switch retire old tasks.
- [x] Verify receive/send with two real WASM providers, renewal, late events,
  reconnect and uncertain delivery receipts without repeating external writes.

### SHARED-06: Server authorization and personal identity — migrated

Depends on SHARED-01, SHARED-02 and SHARED-05. Owner: workspace vault, auth UI and guests.

User/device identities, own-device linking and isolated vault slots are retained.
Former server-hosted personal accounts and account copying are replaced by
independent device/server authorization, direct device actions and an explicit
legacy-copy cleanup path.

- [x] Separate server-local users from devices; linking is explicit and does not
  infer identity from names or workspace membership.
- [x] Replace the former server-hosted personal-account plan with SHARED-09–13.
  Do not expand per-user server credential uploads, personal execution or delegation.
- [x] Authorize the server part independently through extension-owned flows;
  it may use a different platform account from every client. Preserve an existing
  working server binding until an explicit successful replacement/disconnection.
- [x] Keep device-local platform authorization independent on every device;
  linked user identity never implies credential copying or shared platform rights.
- [x] Migrate existing server-held personal bindings through an explicit safe
  disconnect/reauthorization flow. Never download secrets or delete the only
  recoverable copy before the user chooses and completes the transition.

### SHARED-07: Install extension packages through authorized clients

Depends on SHARED-02, SHARED-05 and SHARED-09. Execution order: second, before
SHARED-10–13. Owner: package installer and generic settings UI.

- [x] Separate upload/install/configure/execute grants. Bound archive expansion,
  reject traversal/symlinks and validate manifests, WASM and contract versions.
- [x] Install only the declared client/server parts on their selected host under
  SHARED-09; connecting a client must not implicitly install or execute packages.
- [x] Persist accepted packages and expose generic settings without granting new
  capabilities implicitly. Failed upgrades preserve the previous usable installation.
- [x] Test malicious archives, incompatible packages, unauthorized installation,
  replacement cancellation and two independent provider packages.

### SHARED-08: Qualify the shared studio workflow

Depends on SHARED-04, SHARED-06, SHARED-07 and SHARED-09–13. This is the final
qualification gate in the migration order, not a completed foundation task.

- [x] Open the deployed UI on a physical phone through trusted HTTPS; verify the
  native keyboard and commit/dismiss the input editor on blur or an outside tap.
- [x] Authorize a real Twitch server account and verify that an operator-sent
  platform message appears once on both desktop and physical phone in shared chat.
- [x] Send a message from the desktop personal account and confirm that it appears
  exactly once on the platform. Physical-phone personal sending remains to verify.
- [ ] Verify desktop plus phone through trusted HTTPS: viewer, chat moderator,
  scene operator and administrator roles, including denied direct API calls.
- [ ] Verify two devices linked to one Acheflow user with different local platform
  accounts: shared receive from the server, direct sends with correct attribution,
  denied Acheflow/platform rights, and no account or credential synchronization.
  Personal authorization must not create another collector or widen server access.
- [x] Qualify future moderation separately once provider guests implement it;
  missing moderation is not a blocker for the receive/send migration.
- [x] Verify restart, session revocation, workspace switch and one provider
  connection per binding; document backup/restore and operator trust.
- [x] Run the affected development checks and real-WASM lifecycle tests; update
  current guides to describe only implemented behavior.

Automated qualification includes real WASM receive/send isolation, archive attacks,
permission denial/revocation, two independent collectors, native/browser builds,
and actual loopback FFmpeg/libx264 and VideoToolbox RTMP video/audio delivery.
The production-server HTTPS integration also covers trusted test certificates,
cookie pairing, distinct device grants, revocation and collector restart recovery.
Physical-phone HTTPS loading, keyboard dismissal and real Twitch shared receive
have been verified separately, including server OAuth consent and account storage.
These checks do not prove a public broadcast, the complete physical-device role
matrix, physical-phone personal sending or live token renewal. Other providers still need
live authorization/receive checks. Complete the remaining checks with the operator's
own accounts before calling SHARED-08 finished. Do not publish test chat or go live
automatically.

### SHARED-09: Declare client and server extension parts — complete

Owner: extension API/SDK, manifests, loaders and contract. Dependencies: SHARED-05.

- [x] Define part-specific entry points, settings, auth, storage, capabilities and
  supported actions. Support client-only, server-only and dual-part packages
  without provider-specific host branches; select a part explicitly at load time.
- [x] Specify compatibility/versioning for existing single-side manifests and the
  trusted provider loader. Migrate repository guests and build their WASM before
  host checks; reject unsupported combinations with actionable errors.
- [x] Test discovery/loading with two real WASM providers and prevent a part from
  obtaining the other host's credentials or capabilities.

Verification: workspace and guest tests, native/browser Clippy, web bundling and
macOS artifact-copy tests pass with real provider WASM. Package installation,
server-only collection and the settings/account migration remain separate tasks.

### SHARED-10: Collect shared chat only on the server

Owner: server provider reactor, client execution lifecycle and live feed.
Dependencies: SHARED-09 and SHARED-07.

- [x] Retain one server collector per enabled binding/provider/channel; remove
  client chat subscriptions, normalized-chat publication and automatic collector
  startup. Retain client execution for actual device integrations/actions.
- [x] Read the authenticated common feed on desktop/browser/phone. Missing server
  authorization is a visible disconnected state, never a local collection fallback.
- [x] Verify two clients and two real providers: no duplicate subscriptions,
  reconnect, disable, replacement, server restart and workspace cancellation.

### SHARED-11: Execute personal actions on the originating device

Owner: client runtime, chat composer, provider guests and action permissions.
Dependencies: SHARED-09 and SHARED-10.

- [x] Resolve a shared channel to a provider target through the extension/domain
  contract, validate current Acheflow grants, then send directly with this device's
  validated local account. Reuse read-only preparation and bounded action execution
  where applicable; opening a sender must never start collection.
- [x] Preserve sender attribution, cancellation, account/workspace fencing and
  uncertain-delivery receipts; never blindly resend or fall back to a server account.
- [x] Replace the personal composer endpoints and retire server personal-account
  storage/execution APIs after the explicit SHARED-06 migration. Keep server
  service-account actions separate from the personal sending path.
- [ ] Qualify live native/browser provider accounts and browser CORS support on the intended devices.
  The action-only WASM paths for all three bundled providers and failure handling
  are tested; external OAuth consent/renewal and physical browser permissions are
  not inferred from synthetic protocol responses.
  Report unsupported device actions explicitly; do not work around restrictions by
  uploading personal tokens to the server. Keep future moderation capability-gated.

### SHARED-12: Finish vault ownership and Keychain access behavior

Owner: local/server vaults and device-session persistence. Dependencies: SHARED-11.

- [x] Use one workspace credential container on each executing host with separate
  extension/application/account slots. A server is its own trust boundary even when
  client and server run on one Mac. Never merge host access merely to reduce prompts.
- [x] Specify the device-to-server session's separate server-scoped lifetime;
  document that it is not a provider credential or a per-extension record.
- [x] Avoid repeatedly reopening the same container per extension. Reuse an unlocked
  session safely while preserving cross-process locking, refresh, revocation and
  isolated writes. Record count and OS permission prompt count are separate criteria.
- [x] Retain client authorization panels across scope changes, reuse bound storage,
  persist retirement of per-extension legacy records and skip unchanged vault writes.
- [x] Label host containers with the workspace name, a shared stable identity suffix
  and Client/Server role; keep full storage IDs independent of display names.
- [ ] Migrate old namespaces without losing tokens or neighboring slots. Verify
  locked/denied Keychain, two providers, restart, concurrent client/server processes
  and actual prompt behavior using the intended signed build; do not promise one
  lifetime prompt or assume a particular cause without real-system evidence.

### SHARED-13: Show host-specific settings and authorization

Owner: extension cards, account UI and server permission checks.
Dependencies: SHARED-09, SHARED-11 and SHARED-12.

- [x] Restore **Client / Server** tabs based on installed/available extension parts.
  Remove **Studio chat / My messages** and the extra **Shared server account** copy
  panel after migration; show each part's own account, settings, state and errors.
- [x] Client authorization belongs to this device; server authorization belongs to
  the server and can select a different account. Opening settings never copies tokens.
- [x] Apply view/configure/install/execute grants to server UI and every underlying
  endpoint; deny direct unauthorized calls and redact secrets from readable settings.
- [x] Verify mouse/keyboard tabs and preserve public drafts when switching scopes;
  show collection and device-local sending separately. Browser visual checks and
  native layout/IME bridge regressions pass.
- [x] Verify physical-phone keyboard entry and dismissal through trusted HTTPS.
- [ ] Finish physical-phone pairing/role and real-account checks in SHARED-08;
  viewport emulation is not physical-device evidence.
- [x] Implement explicit hosted/direct browser callback selection, hosted-route
  disclosure and in-memory return to the original HTTPS studio window. Preserve
  separate native app links and provider-owned OAuth state/account validation.
- [x] Use a shared callback endpoint and versioned transport envelope for native
  and browser clients; keep workspace/instance ownership in the waiting client.
  Support non-OAuth carrier fields without adding provider branches to the host.
- [x] Add separate client/server Callback proxy URL preferences for custom HTTPS
  hosting. Keep client values device-local; permission-check, revision-check and
  persist server changes without moving credentials.
- [x] Publish the matching browser callback pages with no-store, no-referrer and
  isolated scripts, alongside the current hosted web client.
- [x] Qualify browser return through the hosted proxy on a physical phone, including
  provider token validation and replacement of Connect by Disconnect.
- [x] Publish the unified native/browser callback router alongside the matching
  hosted web client.
- [x] Use `/callback/` as the shared public endpoint, support contextual
  `/callback/*` paths and remove the old public OAuth-specific route. Verify a
  real desktop sign-in returns to the installed client and shows Disconnect.
- [ ] Qualify the unified native/browser envelope and custom-host configuration;
  verify direct return, cancellation and disconnected windows on
  physical devices. Complete phone personal sending after connection recovery.
- [ ] Provision a shared Acheflow provider application before offering sign-in
  without a user-supplied Client ID. Keep custom applications available; do not
  invent or silently substitute application credentials.
- [ ] Evolve the hosted web client into an entry point for selecting remote
  servers. Proxying server traffic or traversing NAT is a separate future feature.

## Existing-logic backlog

These tasks improve existing behavior. Code-review findings need a regression
reproduction before implementation; timing risks need measurements before choosing
a correction. Unchecked items are open. P1 covers state consistency and lost edits;
P2 covers recovery and completion of existing mechanisms; P3 covers maintainability.
New bugs found during everyday use can change this order.

Continue with the remaining P2 items. Keep each task a
separate reviewable change. For every implementation, run the affected regression
tests and applicable [development checks](development.md#standard-checks), update
the owning guide, and only then mark the task complete. Keep provider-specific
behavior under `extensions/` and follow the [extension contract](extension-contract.md).

### LOGIC-01: Atomic extension configuration — complete

Enablement and public settings share one durable edit. Validation, guest loading
and saving must succeed before the host replaces a running instance.

Owner: [control](../crates/acheflow-server/src/control.rs),
[workspace extensions](../crates/acheflow-server/src/workspace_extensions.rs),
[guest lifecycle](../crates/acheflow-server/src/extension_host.rs). Dependencies: none.

- [x] Validate the installation and requested configuration before mutating settings.
- [x] Define and enforce one consistent failure outcome for settings, runtime status
  and revisions; never leave a failed startup reported as a running instance.
- [x] Cover unknown installations, invalid configuration, guest-load failure and
  successful enable/disable with regression tests using real WASM guests.

### LOGIC-02: Protect all workspace edits from stale Apply — P1

Completed with SHARED-03: every saved configuration edit has a durable revision.
Stale Apply is rejected for media, layers, audio, outputs and extension settings.

Owner: [workspace application](../crates/acheflow-server/src/live.rs),
[client synchronization](../crates/acheflow-ui/src/control_client.rs). Dependencies: none.

- [x] Reproduce conflicting edits from two clients for layers, audio routing and
  output settings, then define the expected-revision and explicit resolution rules.
- [x] Reject or explicitly reconcile stale Apply without silently discarding another
  client's edits; preserve unrelated local drafts and live-edit restrictions.
- [x] Test simultaneous edits, accepted conflict resolution and server restart;
  verify the conflict is understandable in the UI.

### LOGIC-03: Ignore stale control responses — P1

Completed with SHARED-04: generation and identity checks discard delayed
snapshot/command responses. Request deadlines release busy state without retrying writes.

Owner: [control client](../crates/acheflow-ui/src/control_client.rs). Dependencies: none.

- [x] Add request-generation or equivalent ordering checks so a delayed snapshot
  cannot undo a newer command response or lower the acknowledged edit revision.
- [x] Bound requests and discard responses from a previous workspace/server;
  recover busy state after timeout and allow a restarted server to establish new state.
- [x] Test delayed GET/command responses, workspace switching and timeout recovery;
  manually verify native/browser controls remain usable.

### LOGIC-04: Reclaim inactive executor registrations — complete

Retired registrations release active slots while retaining generation history
for a bounded window. Withdrawing a workspace releases its loaded policy.

Owner: [execution registry](../crates/acheflow-control/src/client_execution.rs),
[registry tests](../crates/acheflow-control/tests/client_execution.rs). Dependencies: none.

- [x] Reclaim inactive registration capacity using a bounded retention policy.
- [x] Preserve generation/replay barriers and cancellation or Unknown outcomes for
  instructions belonging to withdrawn authority; apply bounded retention to policies too.
- [x] Test more than 64 successive expired clients, active-capacity limits, stale
  generations, retained instructions and isolation between workspaces.

### LOGIC-05: Recover from bounded audio lag — complete

Replace up to 160 ms of missed PCM with silence per recovery so the encoder's
sample timeline stays continuous. A destination that exceeds the budget fails
independently.

Owner: [output worker](../crates/acheflow-server/src/output.rs),
[audio mixer](../crates/acheflow-server/src/audio_mixer.rs). Dependencies: none.

- [x] Reproduce lag with a deliberately slow consumer and distinguish Lagged,
  channel closure, cancellation and socket-write failure.
- [x] Define a bounded resynchronization policy with an explicit unrecoverable
  threshold; do not accumulate latency or silently corrupt A/V timing.
- [x] Verify short stalls recover, persistent failures remain visible, and another
  destination keeps delivering media during the affected output's recovery.

### LOGIC-06: Reconnect failed destinations — complete

Recognized transport failures retry per destination with bounded backoff. Studio
shows the attempt and any terminal failure.

Owner: [destination lifecycle](../crates/acheflow-server/src/compositor.rs),
[output worker](../crates/acheflow-server/src/output.rs). Depends on LOGIC-05.

- [x] Add bounded retry/backoff for recoverable failures with visible attempt status
  and an explicit terminal outcome for permanent errors or exhausted retries.
- [x] Ensure Stop, deactivation and workspace changes cancel pending retries and
  retire old workers before replacement; manual restart must not create duplicates.
- [x] Test connection loss/recovery with real FFmpeg output and two destinations,
  including cancellation during backoff.

### LOGIC-07: Measure sustained A/V timing — complete

The [loopback marker probe](development.md#sustained-av-timing) found offsets
outside the provisional budget, including after an independent output restart.
Both outgoing tracks kept continuous packet timestamps around the injected
source jump, while the relative marker offset changed. Investigate the audio
PTS rebase, the compositor's latest-frame selection and the independent audio
and video encoder feed clocks in LOGIC-08.

Owner: [ingest](../crates/acheflow-server/src/ingest.rs),
[audio playout](../crates/acheflow-server/src/audio_mixer.rs),
[media tests](../crates/acheflow-server/src/pipeline_tests.rs). Dependencies: none.

- [x] Build a repeatable real-media check with synchronized visual/audio markers;
  measure offset, drift and delivery continuity at the receiving output.
- [x] Exercise a sustained baseline, CPU/GPU load, source reconnect, timestamp
  discontinuities and independent output restart; define duration and tolerances.
- [x] Identify whether a timing correction is needed and the owning clock/queue;
  keep raw recordings and environment-specific results outside tracked docs.

### LOGIC-08: Correct measured timing failures — complete

The marker trace located a slow clock reacquisition after reconnect and delays
in vertical composition under load. Auto sync now acquires from several fresh
presented frames before playing audio and bounds subsequent clock corrections.
Encoder feeds share an epoch and preserve short gaps with silence or repeated
frames. The video feed uses render time to decide which image fills each slot.
GPU readback converts packed pixels by row to reduce CPU time at larger sizes.
Studio exposes signed manual trim, per-track sync status and per-format load.
The loopback probe passed its steady-state gates under load, after a source PTS
jump, after publisher reconnect and after restarting the vertical output. It
passed with a 30 fps source and horizontal output and a vertical output at both
30 fps and 25 fps.

Owner: the ingest, mixer, compositor or output path identified by LOGIC-07.
Depends on LOGIC-07.

- [x] Specify the failing timeline transition and the expected timestamp, frame
  drop/duplication or silence policy before implementing a correction.
- [x] Keep queues and latency bounded across source/output reconnects and load;
  preserve relative timing between a source's audio tracks.
- [x] Pass the same baseline and stress checks used to reproduce the failure,
  including both output orientations.

### LOGIC-09: Own executor registration in Studio — P2

Historical implementation: native Studio owns local runtimes and registration.
Client chat collection is superseded by SHARED-10. Retain lifecycle fencing for
device integrations and direct actions; do not expand local shared-chat collectors.

Owner: [executor bridge](../crates/acheflow-client/src/bridge.rs),
[account lifecycle](../crates/acheflow-ui/src/client_authorization.rs). Depends on LOGIC-04.

- [x] Tie native executor registration, heartbeat and bounded reconnect to the active
  workspace/server and authorized local account session.
- [ ] Withdraw authority on account removal, disable, workspace switch and shutdown;
  make advertised capabilities and availability reflect current live authority.
- [ ] Verify two independent real WASM providers, expiry/generation renewal and
  cancellation; preserve provider-neutral host/UI code.

### LOGIC-10: Run provider events and instructions continuously — P2

Historical implementation: Studio schedules native receive/publication and claims.
Shared-chat receive/publication moves exclusively to SHARED-10; client actions move
to SHARED-11. The checks below apply to the appropriate executing host, rather than
requiring a second client chat runtime. Account setup does not prove availability.

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

The server persists its active workspace configuration; live session/chat history
and execution receipts remain in memory. Audio output is one AAC stereo track per destination,
with no local monitoring playback. Final composition copies frames to CPU before
FFmpeg. HDR/color management, glTF/GLB import, PBR,
lights and authored animation are not implemented as complete product workflows.

A later managed bot/AI offering is optional. Its extension parts must follow the same explicit host ownership, grants and
credential boundaries; personal client credentials must not be uploaded implicitly.
Self-operated installations remain a supported architectural goal.
