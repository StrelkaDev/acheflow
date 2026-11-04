# Desktop lifecycle and forwarding tasks

Details and acceptance criteria: [plan-desktop-links.md](plan-desktop-links.md).

- [x] DL-01: Reject all duplicate extension IDs and report their sources.
- [x] DL-02: Define and parse client/server `/fwd/<extension_id>/<action>/...` links.
- [x] DL-03: Deliver manifest-declared forwarding events with lifecycle isolation.
- [ ] DL-04: Receive native client links on cold and warm startup.
- [ ] DL-05: Receive native server links without moving provider auth to the server.
- [ ] DL-06: Prepare OS handlers, domain association and HTTPS fallback pages.
- [x] DL-07: Add complete provider-owned application setup instructions and controls.
- [ ] DL-08: Use YouTube streaming chat with resumption and cancellation.
- [ ] DL-09: Launch and monitor an owned local server process.
- [ ] DL-10: Add desktop Settings / Server controls and lifecycle preferences.
- [ ] DL-11: Package matching client/server binaries and installation instructions.
- [ ] DL-12: Later register official provider applications and request needed quotas.
- [ ] DL-13 (future): Use `acheflow://` with explicit recipient routing and one executable supporting server-only or client with an on-demand server.

## Checkpoints

- [x] Forwarding: catalog ambiguity, parser and two real guest delivery tests pass.
- [ ] Authorization/chat: guest rebuild, isolation and streaming regressions pass.
- [ ] Desktop: owned-process lifecycle and native/browser settings checks pass.
- [ ] Distribution: package artifacts and local activation smoke tests pass.
- [ ] External rollout: domain/signing/provider setup verified with actual access.

## Initial implementation — 2026-11-04

DL-01 covers native/bundled client catalogs and the server catalog. All conflicting
copies are removed; a third copy cannot restore the ID or replace browser auth.
Diagnostics identify directories/bundled slots. A truncated/unreadable directory
scan disables the catalog instead of admitting unresolved duplicates.

DL-02 provides `acheflow_extension_api::forwarding::ForwardLink::parse_for` and the
[address contract](../docs/forwarding.md). It validates the expected recipient,
manifest ID, action, encoding, canonical path and size; diagnostic formatting omits
callback payloads. Native activation and live guest delivery are still DL-03–06.

Verification: duplicate regressions reproduced the original failure, then passed;
native/bundled client and server fixtures use real provider WASM. Workspace tests:
192 passed, 0 failed, 11 existing ignored. Native Clippy for the affected packages,
common/client WASM Clippy (`-D warnings`), formatting and whitespace checks passed.
Repowise was refreshed; after simplifying the discovery paths, its live change
review has no high-severity findings. Remaining medium findings concern parser/
catalog branch counts and bounded test-fixture I/O. No live provider registration
or domain activation was tested.

## DL-03 implementation — 2026-11-04

Manifests declare unique event/callback actions. Local forwarding verifies the
active workspace/instance and, for callbacks, a single-use host-issued correlation
with bounded expiry. Credentials are captured before guest delivery. Ordinary
events cannot modify staged authorization secrets. Stop, disconnect, first workspace
binding and restart revoke the relevant grants; failed/stopped sessions refuse late
results. Existing local authorization before an explicit local_start remains valid.

Server forwarding uses the normal workspace/revision/request-dedup control boundary
and private bounded queues belonging to a specific live guest instance. Snapshots
expose the instance ID; replacement instances cannot receive old queued events.
Provider callback interpretation remains in the guest SDK, with source and WASM
artifacts rebuilt together. Both local and server deliveries carry request identity.

Verification: 209 workspace tests passed, 0 failed, 11 existing ignored; guest
workspace 4 passed. The affected client/server suites were rerun after the final
request-identity change and passed. Native workspace/all-target and UI/client WASM
Clippy passed with -D warnings, along with formatting and whitespace checks.
Real Twitch/YouTube guests cover isolation, credential staging, expiry, replay,
disconnect, restart and pre-bind requests; two real server guest instances cover
targeting, stopped/replaced instances, workspace checks and request idempotence.
The Event credential-substitution regression failed before its fix and passed after.

Repowise was refreshed. Its review still flags the existing complex manifest
validator and medium parser/reactor complexity; the new validation, correlation and
credential boundaries were reviewed and exercised by the tests above. Native OS
activation, signed domain association and live platform consent are still DL-04–06.

## DL-04 implementation — 2026-11-04 (verification pending)

Added `acheflow-launcher` with per-recipient single-instance election, a private
bounded Unix IPC inbox, stale-socket recovery and delivery receipts. The native UI
routes activation to enabled installed sessions, focuses its window, selects
Extensions and reports missing/conflicting/disabled recipients. Requests retain
workspace/configuration/instance identity while waiting; expired or cancelled
requests cannot reach a replacement session. Pending browser operations accept
correlated forwarding without cancelling the legitimate wait on invalid input.

Added a thin Swift URL-event adapter and a development macOS bundle builder. The
main `.app` owns the GUI; a separate nested helper passes the URL through stdin.
Neither helper nor Rust diagnostics log callback URLs. Signed domain association
and production handler registration remain DL-06.

Verification: native launcher/UI all-target Clippy and UI WASM Clippy passed with
`-D warnings`; Swift type-check and the development bundle build passed. A queued
request starvation regression was reproduced and passed after its fix. New IPC
and UI mailbox suites compile, but their complete runtime execution remains
unverified: sandbox restrictions prevented socket listening and the requested
outside-sandbox process-suite run was declined. A native visual smoke check is
also pending, so DL-04 remains unchecked. The isolated test app was stopped.

## DL-05 implementation — 2026-11-04 (process verification pending)

The server accepts `--native-links` for an explicitly enabled local receiver and
`--forward-stdin` for URL handoff/cold start. Ordinary standalone startup remains
unchanged. The receiver shares the private IPC/election implementation but owns a
separate server endpoint. It starts after server listener/control initialization;
an unconfigured fresh server rejects links rather than creating a workspace or
enabling an extension.

Each event captures the current workspace, revision and live guest instance and
uses the existing control command boundary. Expiry/cancellation is rechecked after
both command and guest-inbox lock waits. No remote forwarding endpoint, provider
authorization or shell execution was added. Shutdown interrupts native waits.

Eight forwarding tests with two real WASM guests pass, including native targeting,
wrong recipients, missing/disabled actions, expired requests, queued workspace
switches and existing restart/idempotence boundaries. The command-lock expiry
regression failed before its fix and passed afterward. The complete local-process
activation suite remains unverified under the restrictions recorded for DL-04;
DL-05 remains unchecked until that acceptance check is available.

The six existing control-service tests also pass. Native server all-target Clippy,
CLI help/build, native launcher/UI Clippy, UI WASM Clippy, Swift type-check,
formatting and whitespace checks passed. No dependency versions were changed;
the lockfile additions register the new local crate and its consumers. Repowise
was refreshed: the new native activation paths have no high-severity findings
after simplifying the UI event driver. Remaining medium findings concern branch
counts in the URL adapter, main-thread router and instance election.

## DL-06 local artifacts — 2026-11-04 (external rollout pending)

- [x] Normalize only the recipient's own platform URI scheme into canonical HTTPS.
- [x] Prepare unsigned macOS client/server URL adapters with separate identities.
- [x] Prepare Linux desktop entries and a fixed-executable stdin adapter.
- [x] Generate separate fallback sites and optional matching Apple AASA/entitlements.
- [x] Verify explicit-click/history cleanup behavior and desktop/narrow layouts.
- [x] Document signing, profile, hosting, registration and current limitations.
- [ ] Verify signed installed handlers, live domains/AASA and full process activation.
- [ ] Implement and validate Windows activation before advertising Windows handlers.

Details: [native-links-installation.md](../docs/native-links-installation.md).
Current custom schemes are transport aliases only; canonical HTTPS delivery and
all manifest/workspace/instance/correlation checks remain in force. No domain was
published, identity signed or user's scheme handler registered. Eight focused
Rust/JS/Python checks pass; native affected Clippy/builds and Swift checks pass.
The static fallback was visually checked at desktop and 320px widths, including
invalid-origin rejection and a dummy valid-link fixture with stubbed OS navigation.
The full task remains unchecked until external acceptance is available.

The user's follow-up is recorded as future DL-13: one `acheflow://` scheme with
explicit recipient routing and one binary with server-only or client/on-demand
server modes. Its exact URL grammar and flags are deferred to that task.

## DL-07 implementation — 2026-11-04

Added a typed `application_setup` extension API contract. Console links,
native/browser instructions, active callback/origin selection, public setting IDs
and required native secret IDs belong to each provider manifest. Validation rejects
unknown/duplicate bindings, unsafe links, excessive metadata and browser secrets.
Real provider guests emit generic controls; the UI resolves fields by the current
installed/enabled extension ID and declared setting ID, preserving local settings.
No provider names, OAuth URLs or provider dispatch tables were added to host code.

Twitch, YouTube and VK Video Live now supply their own setup. YouTube browser uses
the actual JavaScript origin and no application secret; native keeps Desktop
loopback + PKCE and reports a missing local secret before opening the browser.
Address copying supports native/browser clipboards and inline failure messages.
Public setting and secret drafts retain their existing workspace/lifetime boundary.

Verification: 235 workspace tests passed, 0 failed, 11 existing ignored; 5 guest
tests passed. Real WASM checks cover three providers on both environments, exact
addresses, flow selection, permissions and setup failures. UI tests exercise two
independent guests, ID-bound settings, keyboard controls, clipboard failure and
320/1000px layouts. Native/browser builds, all-target workspace/guest Clippy,
UI WASM Clippy with `-D warnings`, formatting and whitespace checks passed.
Native and actual WebGPU browser setup panels and successful Copy controls were
visually checked using isolated test workspaces. The existing native route test
fixture now enters the app scope where its context is provided.

Repowise review prompted smaller validation methods. Remaining findings concern
test length and moderate validator branching. No live account consent or provider
application registration was performed. Unified URI/binary work remains DL-13.
