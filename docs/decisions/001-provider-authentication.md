# ADR-001: Separate server bot credentials from personal client authorization

## Status

Superseded by [ADR-002](002-client-execution-and-workspace-capabilities.md),
2026-11-03. The server-held bot credentials and server-side platform execution
described below are historical decisions, not the current target architecture.

This decision governs the authentication boundary in the
[extension contract](../extension-contract.md). It supersedes server-side handling
of personal account tokens and personal message sends in the earlier design.
Provider-specific behavior continues to belong to extensions.

## Context

Acheflow is open source and can run on independently operated servers. Each
operator must be able to configure their own platform applications, bot identities
and AI services. There is no shared production token to distribute in the source
or compiled artifacts. OAuth application registration, a bot account, channel
moderator status and OAuth permissions are distinct concerns.

The owner requires personal authorization and personal message sending to stay on
the client. A future managed bot/AI service is a separate, optional deployment.

## Decision

### Credential ownership

| Data or action | Owner |
|---|---|
| Server bot OAuth application configuration and private application secrets | Server operator |
| Bot access/refresh tokens | Server-side private credential store |
| Enabled extensions, selected channels, bot connection references and automation rules | Workspace configuration |
| Personal account authorization and access/refresh tokens | Local client credential store |
| Personal chat messages | Client sends directly to the platform API |
| Collection, bot replies and automatic moderation | Server bot, with the required platform permissions |
| AI provider configuration and credentials | Operator of the selected bot/AI service |

No access token, refresh token, private client secret or AI service key may be
embedded in source, guest artifacts, examples or default configuration. A public
OAuth client ID identifies an application; it is not an account token or a secret.
Administrator documentation must distinguish these values.

### Independently operated servers

Operators register and configure their own platform applications where required,
authorize the bot identity, and grant channel roles needed for enabled features.
Collection-only permissions must not imply moderation permission. Documentation
must explain account-based and application-token collection separately for each
provider; a separate bot account is not a universal requirement.

Server bot credentials remain private to that server and provider/account
connection. Workspaces select connection references, channels and enabled
capabilities rather than carrying credential values. Reusing a configured bot
connection must not allow one workspace to act on another workspace's targets.

### Personal client connection

The client provides Connect account, opens the platform authorization page and
obtains the user's grant using a flow supported for that client type. The client
keeps personal tokens locally and sends personal messages directly to the platform.
Neither the tokens nor a token-bearing proxy request is sent to Acheflow servers,
including a future managed service. The application must identify the sender
account before sending.

Local credential identity must distinguish the provider, OAuth application and
authorized account. Selecting another server/workspace must not silently select a
different sender or disclose credentials. Workspace exports contain no tokens.
Clients use trusted client-side provider adapters; remote servers must not supply
arbitrary executable authorization code or token-bearing request destinations.
Provider protocols remain outside the shared UI and server core.

Native and browser OAuth grants, callback handling, renewal and secure local
storage must be verified per provider before implementation claims support.
If a platform cannot support the required client-only flow, expose that limitation;
do not silently exchange or relay personal tokens through an Acheflow server.

### Optional managed bot and AI service

A later service may operate bots, collection, automation, moderation and AI on
project-operated hardware and connect to an Acheflow server. Alternatively, an
operator runs their own bot and their own AI infrastructure. The managed service
must not be required for the independently operated mode or personal client login.
Bot credentials belong to the selected operator in either mode. Sending chat or
other content to a managed/remote AI service requires an explicit configuration;
choosing the self-operated mode must not silently forward data there.

The managed service, AI implementation, billing and deployment infrastructure are
future work, not part of the current authentication change.

## Alternatives considered

- Shipping a common bot token or private application secret: rejected because
  independent installations cannot safely share or control those credentials.
- Keeping personal tokens on every Acheflow server: rejected by the owner; personal
  authorization and personal sends must stay on the client.
- Requiring a central project service: rejected because independently operated
  deployments must remain functional with their own bots and AI configuration.

## Consequences and verification

The current implementation has one server-side token store and server-routed
chat sends. YouTube/VK currently accept manually entered tokens. These paths are
legacy implementation, not evidence that the client-only boundary exists. A
manual server token must not be presented as the new personal Connect flow.

The remaining work includes a client provider runtime/authentication path, local
credential handling, direct sends with delivery/echo reconciliation, and provider
administrator guides. The shared channel UI must distinguish personal sending
from server bot capabilities without exposing provider protocols in the core.

Before declaring migration complete, verify with two independent providers:

- Personal OAuth, token exchange/renewal and sends remain on the client; no personal
  tokens reach server requests, events, logs, snapshots or exported workspaces.
- Independent servers and workspaces retain separate bot configuration and channel
  permissions; sender selection remains explicit when switching context.
- Bot moderation requires both applicable channel roles and OAuth permissions;
  personal sending does not depend on the server holding personal credentials.
- Cancellation, logout, restart, revoked grants and late results cannot restore a
  disconnected identity or repeat an outgoing message.
- Independently operated mode works without the managed service, and AI content
  is sent only to the explicitly selected service.

Provider fixtures and sandbox tests do not replace live account validation.
