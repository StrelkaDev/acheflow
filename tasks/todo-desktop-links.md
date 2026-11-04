# Desktop lifecycle and forwarding tasks

Details and acceptance criteria: [plan-desktop-links.md](plan-desktop-links.md).

- [x] DL-01: Reject all duplicate extension IDs and report their sources.
- [x] DL-02: Define and parse client/server `/fwd/<extension_id>/<action>/...` links.
- [ ] DL-03: Deliver manifest-declared forwarding events with lifecycle isolation.
- [ ] DL-04: Receive native client links on cold and warm startup.
- [ ] DL-05: Receive native server links without moving provider auth to the server.
- [ ] DL-06: Prepare OS handlers, domain association and HTTPS fallback pages.
- [ ] DL-07: Add complete provider-owned application setup instructions and controls.
- [ ] DL-08: Use YouTube streaming chat with resumption and cancellation.
- [ ] DL-09: Launch and monitor an owned local server process.
- [ ] DL-10: Add desktop Settings / Server controls and lifecycle preferences.
- [ ] DL-11: Package matching client/server binaries and installation instructions.
- [ ] DL-12: Later register official provider applications and request needed quotas.

## Checkpoints

- [ ] Forwarding: catalog ambiguity, parser and two real guest delivery tests pass.
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
