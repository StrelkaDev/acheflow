# Local extension authorization

Each provider owns its OAuth configuration, permission mapping, token validation,
expiry and renewal under `extensions/`. Its real WASM guest emits the account
panel and handles its buttons. Reusable OAuth guest code is in
`extensions/guest-sdk/src/authorization.rs`; the client does not implement an
OAuth controller or choose provider scopes.

The client supplies bounded HTTPS, cryptographic randomness/S256, a browser callback,
generic panel/secret-input rendering and local secret storage. A callback or token
exchange stages credentials. The guest checks state, application identity and
permissions before committing them; a rejected attempt leaves the previous account
intact. Credential values are captured/redacted before guest delivery and never
become Workspace fields, server commands, snapshots or MCP events.

## Setup

Enable an installed extension and expand **Application setup** in its card.
Each provider declares console/documentation links, platform-specific instructions
and public fields bound to its own setting IDs. Enter your application's public
Client ID there and enable only the functions you need. The guest displays the
requested permissions and the exact active callback or authorized origin with
**Copy**. If the clipboard is unavailable, select the read-only address manually.
These declarations use the shared typed application-setup contract; the UI has no
provider list, console URL or provider-specific setting binding.

- Native macOS: callback `http://127.0.0.1:21941/oauth/callback`. An authorization
  attempt owns this loopback listener and opens the system browser. Cancellation
  releases it. Register the callback exactly for Twitch and VK Video Live.
- Twitch: use an application supporting the implicit grant (Confidential type).
  Twitch Public clients are restricted to device authorization. The current
  implicit flow requires no application secret; device authorization is not
  offered by this guest. Verify email and enable 2FA before app registration.
- Browser: serve the bundled `oauth-callback.html` alongside the app and register
  that URI for redirect-based providers. Allow sign-in popups. Reverse-proxy base
  paths are preserved. Browser traffic goes directly to declared provider services;
  CORS failures remain visible and never fall back to server-held credentials.
- Google native: create a **Desktop app** OAuth client, enable YouTube Data API v3,
  and enter the generated application secret in the local extension panel. The guest
  uses an authorization code with S256 PKCE and can renew a returned refresh token.
- Google browser: create a **Web application** OAuth client with the current browser
  origin authorized. `extensions/youtube/youtube-oauth.js` uses Google Identity
  Services' token model. No application secret or refresh token enters the browser.
- VK Video Live: register a DevAPI application in its developer console. VK ID may
  handle the platform sign-in; the documented DevAPI OAuth endpoint still issues a
  separate token. Public chat/audience reading requests no privileged scopes.

Twitch requests chat-read, chat-send and poll-management scopes independently.
YouTube requests `youtube.readonly` for reading and `youtube.force-ssl` when chat
sending is enabled. VK sending/polls are not implemented and are not offered.
Turning off a function also disables the corresponding guest operation. Enabling
new functions validates the existing token against their requirements and requests
reconnection when permissions are missing. Older broader platform consent is not
silently revoked; revoke it in the platform account settings when needed.

Manual access-token entry remains available under **Enter tokens manually** in each
guest's panel. **Application setup** contains provider registration instructions
and the local application-secret field where required. Native panels
can also accept an optional refresh token when their flow supports it. A manual token is validated by the
provider before being committed. Browser panels accept access tokens only. Tokens
without a refresh mechanism require reconnection after expiry.

## Storage and lifetime

On this Mac, committed native credentials use macOS Keychain. Records are scoped to
the selected server, workspace, extension and public application ID. One account is
active per extension/workspace/application slot. Disconnect removes that record;
disable stops the local instance without deleting the saved account. A locked or
unavailable keychain produces an error; there is no plaintext-file fallback. Other
native platforms need their own OS vault adapter before durable use.

Browser credentials live only in memory. They are never written to localStorage,
IndexedDB, workspace export or a server. Disable/workspace/server changes dispose
the local instance and discard browser credentials. Late results cannot restore it.

## Execution boundary and verification

These panels authorize local provider sessions. `LocalSession::install` can restore
their state into a matching native provider runtime in the same workspace. This
does not create an MCP executor credential, register workspace grants, start an
always-on executor or implement a browser media executor. Those remain the separate
client execution boundary described in [client-executor.md](client-executor.md).

Regression coverage exercises the actual provider WASM artifacts, read-only scopes,
disabled sends, callback-state rejection, staged credentials, workspace exports and
real loopback callbacks. macOS Keychain has an explicit real-system roundtrip test
that creates and removes its own isolated fake-token record. Account consent,
provider app registration, live renewal and platform permissions still require
separate testing with configured applications/accounts.

Sources: [Twitch OAuth](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/),
[Twitch app registration](https://dev.twitch.tv/docs/authentication/register-app/),
[Twitch scopes](https://dev.twitch.tv/docs/authentication/scopes/),
[Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[Google browser token model](https://developers.google.com/identity/oauth2/web/guides/use-token-model),
[Google browser client setup](https://developers.google.com/identity/oauth2/web/guides/get-google-api-clientid),
[VK Video Live authorization](https://dev.live.vkvideo.ru/docs/main/authorization),
[VK chat methods](https://dev.live.vkvideo.ru/docs/method/chat).
