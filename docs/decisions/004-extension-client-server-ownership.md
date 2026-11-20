# ADR-004: Extension parts and device-local personal actions

## Status

Accepted target, 2026-11-14; migration is unfinished. See
[the roadmap](../roadmap.md#current-implementation-and-migration-order).
Refines [ADR-003](003-trusted-server-workspace.md): supersedes its server-held
personal-account bindings, personal execution/delegation path and optional client
chat collection. Its shared workspace, device/user identities, permission checks
and trusted server collector remain applicable.

## Context

An extension may need both a server collector and client actions. The current
single-side manifest plus trusted provider loader, local collectors, account-copy
panels and server-held personal bindings blur host and account ownership.
A linked Acheflow user can use different platform accounts on different devices.
Reading the shared feed does not require another platform subscription.

## Decision

- Extensions declare client and/or server parts, their settings, capabilities,
  authorization and entry points. Each installed host executes its own part.
  Provider authentication, targets and protocols stay under `extensions/`.
  SHARED-09 implements `parts_version: 1`, per-host entry points and isolated
  declarations with explicit legacy compatibility; see the
  [package contract](../extension-contract.md#package-parts-and-compatibility).
  Installation and the remaining execution/settings migration are unfinished.
- The server part authorizes an independent platform account and collects the
  common chat feed once per enabled binding/provider/channel. Clients consume
  the authenticated server feed; there is no client collection fallback.
- Every device authorizes its client part separately. Personal sending and future
  moderation execute directly on the platform using that device's account.
  Personal tokens remain local and are not synchronized through an Acheflow user
  identity or uploaded to the server. The server's account may differ from all
  client accounts. Shared-account delegation is outside this migration.
- **Client / Server** tabs configure the corresponding extension parts. Server
  view/configuration/installation/execution require explicit Acheflow permissions,
  enforced on every relevant endpoint as well as in the UI. Readable configuration
  never includes tokens. Viewing chat does not imply permission to send or moderate.
- Client actions check current Acheflow grants and the chosen local account's
  platform scopes/rights for the target. Never silently select another account.
  A locally held token can still be used outside Acheflow: app-level restrictions
  are not a platform-wide revocation mechanism. Supported paths must recheck
  authority at dispatch and fence stale session/workspace/account changes.
- Each executing host owns one workspace credential container with isolated
  extension/application/account slots. Client and server are separate trust
  boundaries even on one machine. A device's Acheflow login is server-scoped,
  with a separate lifetime from provider credentials. Avoid per-extension OS reads;
  safe reuse must preserve cross-process updates and revocation. One container
  does not guarantee one macOS permission prompt.
- Preserve uncertain-write receipts and at-most-once dispatch safeguards during
  migration. Lost acknowledgements must not trigger blind provider retries.
  Browser restrictions remain explicit; unsupported direct operations must not
  silently fall back to sending personal credentials to the server.

## Migration and verification

Keep workspace persistence/synchronization, grants, user/device linking, server
collector ownership and reusable action/receipt primitives. Follow the authoritative roadmap order: SHARED-09 contract, SHARED-07 package
installation, SHARED-10 server-only collection, SHARED-11 direct personal actions,
SHARED-12 vault access, SHARED-13 UI, and SHARED-08 final qualification.
SHARED-07 and SHARED-08 remain unfinished.
Retire server personal-account/send APIs only after replacement and an explicit
reauthorization/disconnect path exist. Do not download stored personal secrets or
remove their sole recoverable copy automatically. Preserve working server account
bindings until explicit successful replacement/disconnection.

Qualify two real WASM providers, desktop and phone, independent device accounts,
a shared user with different device authorizations, denied direct server API calls,
revocation, workspace switch, restart and no duplicate subscription. Test mobile
keyboard input and real Keychain access separately from mocked storage behavior.
Built-in moderation is future work; capability boundaries are required now, actual
moderation support is not required to finish receive/send migration.

## Consequences

Shared chat remains available without a desktop client. Personal actions depend
on the originating device and its platform support; another device needs its own
authorization. A server compromise exposes server credentials but should not expose
personal device credentials through this design. Generic host code and UI remain
independent of individual providers.
