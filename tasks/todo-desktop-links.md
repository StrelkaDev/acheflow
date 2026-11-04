# Desktop lifecycle and forwarding tasks

Details and acceptance criteria: [plan-desktop-links.md](plan-desktop-links.md).

- [ ] DL-01: Reject all duplicate extension IDs and report their sources.
- [ ] DL-02: Define and parse client/server `/fwd/<extension_id>/<action>/...` links.
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
