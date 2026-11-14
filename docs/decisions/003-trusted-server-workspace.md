# ADR-003: Trusted server and shared active workspace

## Status

Partially superseded by [ADR-004](004-extension-client-server-ownership.md),
accepted 2026-11-14. The personal accounts/delegation section below describes the
previous target and transitional implementation. Personal actions now belong to
each device, shared chat collection belongs only to the server, and settings are
split by extension part. Retain the shared workspace and permission foundations.

Accepted as the target architecture. Implementation is incremental; see the
[shared-server tasks](../roadmap.md#shared-server-workspace).
Supersedes the client-only credential ownership decision in
[ADR-002](002-client-execution-and-workspace-capabilities.md) when the server
executor and authenticated migration are implemented. The explicit trusted server reactor is implemented. Explicit trusted-server account copying is implemented: the server revalidates
credentials through the guest before saving an isolated account slot. Local
accounts remain until separately disconnected. Personal accounts now bind to the authenticated user and use separate vault slots,
without starting collectors. Per-action account selection and delegation remain
unfinished.

## Context

Several devices should operate one studio server. Device-local workspace catalogs
and independently connected provider clients do not provide a common project or
one authoritative chat connection. One active workspace per server is sufficient.
The server operator is trusted with the credentials required to execute integrations.

## Decision

- The server owns one active workspace and its saved configuration. A client's
  selected editor view is separate from the server's running state. Connecting or
  opening a view does not replace, deactivate or start a workspace.
- Clients authenticate as distinct, revocable devices. Invitations cannot grant
  more authority than their issuer. Authorization is enforced on server endpoints,
  subscriptions and commands, including existing media/control/MCP surfaces.
- Grants distinguish studio preview, interactive view, chat read, chat send, scene
  control, media start/stop, extension configuration, extension execution,
  installation and access administration. Roles are presets over these grants.
  Viewing a workspace never grants access to stream keys or integration tokens.
- Configuration changes carry the expected server revision. Stale writes preserve
  local drafts and require explicit resolution. Reconnects fetch authoritative
  state; clients must not replay an old configuration automatically.
- The trusted server owns provider sessions, subscriptions, renewal and execution.
  Only one live executor owns each workspace/extension/application/account binding.
  A second viewer cannot start another collector. Ownership transfer retires the
  former executor before activating its replacement, with fencing for late events.
- Provider authentication, endpoints, mappings and subscription logic remain in
  extension WASM. The host remains provider-neutral. Local device features may
  still use client extensions with explicit, separate execution authority.
- A server-scoped workspace vault isolates extension/application/account slots.
  On macOS one workspace uses one OS credential record, rather than one per
  extension. Authorized operations may supply secrets but ordinary settings,
  snapshots, logs, events and workspace exports never return them. Server owners
  can access credentials used by their running server; disk protection is not
  protection from that administrator.
- Package upload, installation, execution and account authorization are separate
  permissions. Uploaded manifests cannot grant themselves host capabilities.
  Extraction, validation, version compatibility and lifecycle replacement precede
  activation. Provider OAuth restrictions determine whether an existing token may
  be transferred or a fresh server authorization is required.

## Shared chat and personal moderation

The server maintains the workspace's primary shared chat feed. Each connected
provider/channel has one collector; opening another client or authorizing a personal
moderation account does not create another chat subscription.

A server-local user identity owns personal provider-account bindings and may have
multiple authenticated devices. Device invitations alone do not prove that two
devices belong to the same person. Linking devices to a user requires an explicit,
authorized flow. This identity layer is implemented: participant invitations create
new users, while own-device links share a user with per-device grants. Legacy
devices migrate to distinct users without invalidating their sessions. Personal provider-account ownership verification is implemented through the guest.
Per-action authority and delegation remain unfinished. Workspace accounts (including a shared bot) and personal accounts
have separate ownership and grants, even when their credentials are stored in the
same trusted server workspace vault.

A moderation command selects one of two explicit authorities:

- Personal: act through the initiating user's verified provider account, with
  permission for the requested operation and target channel on that platform.
- Delegated: act through a workspace/server account only when its owner has granted
  that user the requested operation on the target provider/channel.

Both paths require Acheflow permission, access to the selected account, sufficient
provider authorization/scopes and actual platform permissions. Server delegation
cannot confer rights the selected provider account does not possess. Merely reading
chat or connecting an owner account never grants moderation authority. The server
verifies ownership and delegation; caller-supplied account identifiers are not proof.
Provider-specific verification and operations remain inside extensions.

Account authorization for actions is independent of collection. One extension host
may maintain the shared collector and several account sessions for individual
commands. Revocation must prevent subsequent execution under the revoked authority;
there is no automatic fallback from a failed personal action to an owner's account.
Record the initiating user/device, selected execution account, authority path,
provider/channel, action and outcome without tokens. A server administrator remains
inside the trusted credential boundary described above.

This is target behavior. Current device sessions and provider/account/channel
capability grants do not yet implement verified user-to-account ownership or
per-user delegation; built-in provider moderation remains unfinished.

## Migration and verification

Protect all existing management routes before exposing a shared catalog or secret
submission. Preserve local catalogs and accounts until explicit authenticated
import succeeds. Do not upload credentials or execute packages merely because a
client connects. Restoring saved state must not silently resume external writes.

Test two clients with different grants, stale and concurrent edits, reconnect and
restart, invitation expiry/revocation, redaction and denied direct API calls.
Qualify two independent real WASM providers with ownership transfer, disable,
workspace switch and late callback/event cancellation. An uncertain provider write
must not be repeated merely because its execution receipt was lost.

## Consequences

Shared studio state and provider connectivity no longer depend on keeping one
desktop client open. A server compromise exposes its authorized accounts, so
scoped grants, revocation and protected transport/storage are part of the design.
An untrusted remote server requires a separately trusted executor; it cannot
receive the same credentials under this trust model.
