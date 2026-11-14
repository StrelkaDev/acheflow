# Extension account authorization

Each provider owns its OAuth configuration, permission mapping, token validation,
expiry and renewal under `extensions/`. Its real WASM guest emits the account
panel and handles its buttons. Reusable OAuth guest code is in
`extensions/guest-sdk/src/authorization.rs`; the client does not implement an
OAuth controller or choose provider scopes.

This guide describes device-local authorization. The trusted server executor uses
a separate account namespace and never adopts these accounts automatically. Explicit shared and personal account copying is implemented. Action execution
through personal accounts and per-user delegation remain SHARED-06. See
[server execution](client-executor.md#trusted-server-execution).

The client supplies bounded HTTPS, cryptographic randomness/S256, a browser callback,
generic panel/secret-input rendering and local secret storage. A callback or token
exchange stages credentials. The guest checks state, application identity and
permissions before committing them; a rejected attempt leaves the previous account
intact. Credential values are captured/redacted before guest delivery and never
become Workspace fields, server commands, snapshots or MCP events.

For APIs without scope introspection, a manifest may explicitly set validation
`scope_path` to `null` and use a user-only identity endpoint with `account_path`.
The SDK requires a nonempty account in its successful response; it does not infer
granted scopes. Requested OAuth scopes remain in the consent flow, and the
provider enforces permissions on each operation. Other manifests retain their
declared scope checks.
Account paths should use stable user IDs rather than display names. Positive
integer IDs are normalized to decimal strings before client registration.

## Setup

Enable an installed extension and enter its fields under **Application credentials**.
Expand **Application setup instructions** when registering the application.
Each provider declares console/documentation links, platform-specific instructions
and public fields bound to its own setting IDs. Enter your application's public
Client ID in **Application credentials** and enable only the functions you need.
The guest displays the requested permissions and the exact active callback or authorized origin with
**Copy**. If the clipboard is unavailable, select the read-only address manually.
These declarations use the shared typed application-setup contract; the UI has no
provider list, console URL or provider-specific setting binding.

Enter your own application's **Application Client ID** in the enabled extension
card before Connect. Native YouTube also requires **Application secret**; Twitch
and VK Video Live implicit sign-in do not. The guest declares which fields are
required. Connect remains disabled for missing or malformed inputs. Saved local
secrets show a placeholder and a **Saved locally** note instead of their value.
Leave the input blank to keep a saved secret, or enter a new value to replace it.
A different application ID uses a separate credential slot. After authorization, use the visible **Disconnect account**
button to clear that account and pending callbacks.

Secret and token inputs render only asterisks, including in the native Blitz UI;
masking preserves the original value for submission and normal editing/selection.

No Acheflow application IDs or secrets are inserted automatically. Registering your
own application lets you choose its owner, trust and quota; creating extra Google
client IDs inside the same project does not create separate quota. See the
[provider setup guide](provider-setup.md).

The host supplies `environment.link` as
`https://app.acheflow.com/fwd/<extension_id>`; the extension declares only its
`authentication.native_callback` suffix, e.g. `oauth/callback`. The fields and
instructions come from each manifest; shared UI/server code has no provider
credentials or automatic application defaults.

- Native Twitch and VK Video Live: register respectively
  `https://app.acheflow.com/fwd/twitch/oauth/callback` and
  `https://app.acheflow.com/fwd/vklive/oauth/callback`. The installed app receives
  the link only for a pending, enabled extension with matching state. Deploy the
  HTTPS fallback and verify installed app association before live use.
- Native Google: each attempt owns an available loopback port, independent of
  server ports. The active URI is available after sign-in starts; no fixed
  localhost port is registered for a Desktop application. Cancellation releases it.
- Twitch: use an application supporting the implicit grant (Confidential type).
  Twitch Public clients are restricted to device authorization. The current
  implicit flow requires no application secret; device authorization is not
  offered by this guest. Verify email and enable 2FA before app registration.
- Browser: serve the bundled `oauth-callback.html` alongside the app and register
  that URI for redirect-based providers. Allow sign-in popups. Reverse-proxy base
  paths are preserved. Browser traffic goes directly to declared provider services;
  CORS failures remain visible and never fall back to server-held credentials.
- Google native: create a **Desktop app** OAuth client and enable YouTube Data API v3.
  The guest uses an authorization code with S256 PKCE and can renew a returned
  refresh token. Enter the Desktop client's application secret in local Application
  setup; it is stored with that workspace's credentials. An explicit trusted-server
  connection also copies it so the server can renew the account. It is never sent
  to another browser. The current guest manifest requires this input before
  Connect. Qualify the configured application's token exchange and renewal
  separately from creating its registration.
- Google browser: create a **Web application** OAuth client with the current browser
  origin authorized. `extensions/youtube/youtube-oauth.js` uses Google Identity
  Services' token model. No application secret or refresh token enters the browser.
- VK Video Live: register a DevAPI application in its developer console. VK ID may
  handle the platform sign-in; the documented DevAPI OAuth endpoint still issues a
  separate token. Public chat/audience reading requests no privileged scopes.

Twitch requests chat-read, chat-send and poll-management scopes independently.
YouTube requests `youtube.readonly` for reading and `youtube.force-ssl` when chat
sending is enabled. VK Video Live sending requests `chat:message:send` and
requires an active stream; reconnect an older read-only account after enabling
**Send chat**. For an existing workspace restricted to reading, open
**Extensions → Server → Saved permissions** and choose **Allow sending** for
the account. This changes only that account/channel’s workspace chat permission;
platform authorization is still required. Native VK polls remain unavailable.
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

On macOS, committed native credentials use Keychain. One record contains the
accounts for a selected server address and workspace, with isolated slots for each
extension and public application ID. One account is active per slot. Disconnect
removes only that slot; the last disconnect removes the empty workspace record.
Disable stops the local instance without deleting the saved account. A locked or
unavailable keychain produces an error; there is no plaintext-file fallback. Other
native platforms need their own OS vault adapter before durable use.

Existing per-extension records migrate when that account is next opened or saved.
The new workspace record is saved before the old record is removed; interrupted
cleanup is retried without restoring an older token over the new one. Unused old
accounts remain in their original records until accessed. Native processes serialize
read/modify/write operations with an OS file lock containing no credentials.
Storage is bounded to 64 slots and 4 MiB per workspace, with the existing 256-KiB
per-account limit. An unsuccessful save reports an error and preserves saved data.

Consolidating local Keychain records does not transfer accounts to the server.
The explicit [trusted-server connection](#connect-a-trusted-server-account) below
uses its own server/workspace namespace.

Browser credentials live only in memory. They are never written to localStorage,
IndexedDB or workspace export. They reach a server only through the explicit
trusted-server connection below. Disable/workspace/server changes dispose
the local instance and discard browser credentials. Late results cannot restore it.

## Connect a trusted server account

First open the shared server workspace, authorize the account locally in
**Extensions → Client**, then choose **Connect this account to the server…**.
Select a discovered channel or enter its exact extension channel ID. Review the
trust notice and confirm that the server may store and use this account. The copy
includes committed access/refresh credentials and any application secret needed
for renewal. It does not include drafts, arbitrary local storage or browser callbacks.
Use trusted HTTPS for a remote server; loopback HTTP is allowed for development.

The server revalidates credentials through its installed WASM guest, obtains the
platform account identity and requires it to match the selected local account.
The operation requires all Apply grants plus **Manage device access** and **Read
chat**, rechecked after validation. The workspace ID and expected revision prevent
an account from being connected to a different or concurrently edited workspace.
Validation is bounded to thirty seconds, eight HTTPS effects and two concurrent
checks. No collector runs during account verification.

Only after validation does the server save a fresh isolated account slot and
publish a primary server binding. Initial capability is **chat.read** only. A saved
account starts collection when the studio is active; connection never starts media
outputs. Replacement fences and joins the old collector before using the new
binding. Storage/configuration failure preserves the previous account and local
copy. The local account remains available until explicitly disconnected locally.

**Disconnect server account** removes the server copy after stopping its collector.
Its server-owned channel binding remains reserved and shows authorization required,
so local clients do not silently resume collection. Reconnect explicitly to resume.
macOS uses the OS workspace vault; other server platforms currently fail closed
without an implemented credential-store adapter. An unused or retired slot that cannot be
removed is retained without execution authority and produces a generic server warning.

This flow connects a workspace/server account. Personal accounts use the separate
flow below. Per-action authority, delegation and moderation remain unfinished.
Reading shared chat does not grant access to credentials or permission to act
through the server account.

## Personal accounts on the trusted server

Any authenticated participant can use **Your personal server account** in
**Extensions → Client**. Authorize locally, choose **Save this account for my
profile…**, review the trust notice and confirm. The server revalidates the token
through its installed guest and derives ownership from the authenticated user.
The API accepts no owner/user ID and no caller-supplied storage or vault slot.

Only devices explicitly linked to that user can list, replace or disconnect the
personal binding. An invitation for a new participant creates a different user,
even when device or display names match. HTTP administrators do not inherit access
to other users' personal bindings; the trusted machine operator can still access
its OS vault. A personal connection does not grant device permissions.

There is one personal account per user/workspace/provider. Personal and shared
accounts use disjoint slots in the same server/workspace vault container. Private
ownership, public provider settings and opaque slot references are saved in the
protected access registry, never in workspace JSON, snapshots or exports. The
registry retains at most 256 personal bindings and its existing 1-MiB size bound.
Existing access files without personal metadata load with an empty account list.

Connecting a personal account does not start a runtime, register a channel, change
the workspace revision or capability policy, or restart the shared collector.
Sending/moderation through personal accounts is a separate unfinished step; the
saved account does not yet select the authority of existing chat commands.

A version precondition protects replacement and disconnect from concurrent edits
on linked devices. User/session ownership and the current server workspace are
rechecked after platform validation. Failed vault or registry writes preserve the
previous binding and local account. Successful replacement retires its old slot.
**Disconnect my account** durably removes the binding before deleting credentials;
if the vault cannot delete them, the UI reports that cleanup failed. The old slot
has no personal binding and cannot be selected for actions. Accounts from previous
workspaces can still be listed and disconnected by their owner.

## Execution boundary and verification

These panels authorize local provider sessions. `LocalSession::install` can restore
their state into a matching native provider runtime in the same workspace. This
does not create an MCP executor credential, register workspace grants, start an
always-on executor or implement a browser media executor. Those remain the separate
client execution boundary described in [client-executor.md](client-executor.md).
Choose **Studio chat** to receive chat, or **My messages** to connect a personal
sending account without opening another collector. The card's **Chat connected**
badge reports a live executor connection or a running server provider.
**Authorized locally as …** reports OAuth validation separately. For enabled
Studio integrations, the desktop automatically starts the local provider runtime
after committed authorization when the workspace is active on its server and
execution is permitted. A provider assigned to the server uses its shared
collector. Continuous browser execution is not implemented.

Regression coverage exercises the actual provider WASM artifacts, read-only scopes,
disabled sends, callback-state rejection, staged credentials, workspace exports and
real loopback callbacks. Workspace vault tests cover isolation, migration failures,
storage limits and parallel native processes. macOS Keychain has an explicit
real-system roundtrip test that creates and removes isolated fake-token records. Account consent,
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
