# Provider application setup

Each installation uses applications registered by its user. Acheflow inserts no
maintainer Client IDs, application secrets or test accounts. The current guest
manifest owns its setup links, platform-specific instructions, input requirements
and requested permissions. The enabled extension card is the authority for the
exact callback/origin used by that installation.

## Connect an account

1. Open Extensions / Client and enable the installed provider.
2. Open Application setup instructions and register your application using the
   provider's developer portal. Choose the application type for this client target.
3. Copy the exact redirect URI or authorized JavaScript origin from the card.
   Enter your own public Application Client ID above Connect and the local
   application secret only where the native card requires it.
4. Select needed functions, then Connect and complete provider consent. Missing
   inputs keep Connect disabled. Allow sign-in popups for browser flows.
5. Confirm local authorization status. Use Disconnect account to clear that account
   and pending callbacks. A successful local account is separate from an executor
   connection; see [client execution](client-executor.md).

Client IDs are public application configuration. Application secrets, account
access/refresh tokens and Acheflow transport credentials are different values.
Never put private values in workspace configuration, public docs or server commands.
Use placeholders when sharing a setup example.

## Native and browser targets

| Provider | Native account setup | Browser account setup | Current scope |
| --- | --- | --- | --- |
| Twitch | User-owned application supporting the guest's implicit grant, Client ID and exact HTTPS callback | Client ID and exact browser redirect | Local authorization; native EventSub/chat/audience runtime; continuous executor integration is separate |
| YouTube | Desktop OAuth client, Client ID, local application secret and code/S256 PKCE over an ephemeral loopback callback | Web OAuth client with the current JavaScript origin; Google Identity Services token flow without an application secret | Local authorization; native gRPC chat runtime and viewer sampling; no continuous browser executor |
| VK Video Live | DevAPI implicit-flow application, Client ID and exact HTTPS callback | Client ID and exact browser redirect | Local authorization; native read-only chat/audience runtime; send/poll operations unavailable |

## Twitch

Use the [developer console](https://dev.twitch.tv/console/apps) and
[application registration guide](https://dev.twitch.tv/docs/authentication/register-app/).
The current guest uses an implicit grant, not a device authorization workflow.
Follow its instructions for an application type supporting that flow; no application
secret is requested by this guest. Complete provider email/2FA prerequisites when required.

For the native client, register:

```text
https://app.acheflow.com/fwd/twitch/oauth/callback
```

For a browser client, register the exact callback shown in its card, including
scheme, host, port and any reverse-proxy prefix. It uses the browser callback page,
not automatic native activation. Enabled features request `user:read:chat`,
`user:write:chat` and `channel:manage:polls` as applicable. Provider consent/scopes
do not create a connected executor or make unfinished poll instructions available.

## YouTube

In the desktop client, enable the integration and apply the workspace to its
server. Leave **Video ID override (optional)** empty to discover active broadcasts
for the authorized account. A single broadcast is selected automatically; if
several are active, choose one in **Live broadcast**. The panel displays the
selected title and copyable video ID. Discovery repeats every 30 seconds and
follows a remaining active broadcast when the selected one ends. The bounded
list contains up to 50 active broadcasts. Set the override only to pin a specific
video, including a broadcast outside your account. Switching closes the old chat
stream and discards its pending batch before reading the new chat.


Select your own Cloud project, enable
[YouTube Data API v3](https://console.cloud.google.com/apis/library/youtube.googleapis.com),
and configure its OAuth consent settings. Create clients through
[Google Auth Platform](https://console.cloud.google.com/auth/clients).
When using a Testing audience, add the intended sign-in users and respect its
provider-imposed access/verification restrictions.

For native sign-in, create a **Desktop app** client. Acheflow's current guest
requires its Client ID and local application secret before Connect. The guest uses
code/S256 PKCE; each attempt owns an available loopback port. Do not replace it
with a Web client or the hosted app-link callback. The listener is independent of
HTTP/RTMP server ports and is released on cancellation. Keep the secret in the
native application-secret field; it is never sent to the Acheflow server.

For browser sign-in, create a **Web application** client and allow the exact
JavaScript origin shown in the card. This token-model flow uses a popup and no
redirect URI or browser application secret. A scheme/host/port change means a
different origin. See [Google's token model](https://developers.google.com/identity/oauth2/web/guides/use-token-model).

Read-only functions request `youtube.readonly`; enabling chat sending requests
`youtube.force-ssl`. Native refresh tokens, when issued, are handled locally.
Browser tokens remain in memory and require reconnection after expiry.
Application registration alone does not qualify token exchange, renewal or chat.

## VK Video Live

Use the [DevAPI developer portal](https://dev.live.vkvideo.ru) and its
[authorization guide](https://dev.live.vkvideo.ru/docs/main/authorization).
This integration targets VK Video Live, separate from the general VK video product.
VK ID portal sign-in is distinct from the DevAPI access token.

Register an application supporting the current implicit `response_type=token` flow.
The native callback is:

```text
https://app.acheflow.com/fwd/vklive/oauth/callback
```

Browser clients use their own displayed callback. No application secret is used by
this flow. The channel setting is the public channel slug without its URL. The
guest requests no privileged scopes for its chat/audience read operations.
**Send chat** requests `chat:message:send` and requires a user token and an active
stream. Native polls and automatic token refresh are unavailable. Reconnect after expiry.

## Callback installation and failures

Native Twitch/VK callbacks require matching hosted fallback assets and an installed
handler, or a verified signed HTTPS association. `acheflow://client/fwd/...` is a
transport into the local client, not a replacement URL to invent in provider
registration. Follow [forwarding](forwarding.md) and
[native installation](native-links-installation.md). Browser callbacks remain
separate and preserve the hosting base path.

Wrong application type, unregistered origin/redirect, incomplete consent setup,
missing local secret and declined permissions are setup failures. Use the guest's
bounded diagnostics and provider console; do not paste callback URLs or token
responses into tracked reports. Validate cancellation, duplicate/late callbacks,
workspace/application changes and restart behavior in an isolated environment.

## Quotas and live qualification

Use the quota associated with your own application/project. Additional OAuth client
IDs in one Google project do not create independent project quota. Streaming chat
still has provider quotas; do not shard one use case to bypass limits. Request
increases only from measured usage and the provider's process.

Before relying on a configured integration, verify actual consent, token validation,
required scopes/roles, renewal/expiry and the desired read/send operation. Distinguish
local account restoration from executor registration and continuous collection.
Fixtures prove software boundaries, not live platform access. Keep accounts,
project/application identifiers and private test evidence out of documentation.
