# Extension account authorization

Each provider owns its OAuth configuration, permission mapping, token validation,
expiry and renewal under `extensions/`. Its real WASM guest emits the account
panel and handles its buttons. Reusable OAuth guest code is in
`extensions/guest-sdk/src/authorization.rs`; the client does not implement an
OAuth controller or choose provider scopes.

The **Client** tab authorizes the personal sender on this device. The **Server**
tab authorizes a separate collector account, which may be different. Personal
tokens are never copied to the server. Server authorization runs in a temporary
isolated session until you explicitly save it to the trusted server; it cannot
open this device's credential container. See [server execution](client-executor.md#trusted-server-execution).

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
Client ID in **Application credentials**. Client authorization requests sending
rights; server collection starts with read functions and write functions disabled.
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
`authentication.native_callback` suffix, e.g. `oauth/callback`, for installed-app
delivery. `environment.app_callback_uri` is the selected callback proxy URI. The fields and
instructions come from each manifest; shared UI/server code has no provider
credentials or automatic application defaults.

- Native Twitch and VK Video Live use the same proxy callback as browser sign-in:
  `https://app.acheflow.com/callback/` by default. Register this one address
  in each provider application; per-device or per-workspace addresses are not
  needed. The response's versioned transport envelope identifies the waiting
  client, and the installed app still checks the pending nonce, extension,
  workspace and live instance. Old `/fwd/<id>/...` registrations remain compatible
  with older clients but are not required by the current shared callback flow.
- Native Google: each attempt owns an available loopback port, independent of
  server ports. The active URI is available after sign-in starts; no fixed
  localhost port is registered for a Desktop application. Cancellation releases it.
- Twitch: use an application supporting the implicit grant (Confidential type).
  Twitch Public clients are restricted to device authorization. The current
  implicit flow requires no application secret; device authorization is not
  offered by this guest. Verify email and enable 2FA before app registration.
- Browser redirect sign-in offers **Via callback proxy** and **Direct to this
  studio**. The preference is local to this browser origin. Allow sign-in popups
  and keep the original studio tab open. Provider SDK modules retain their own
  transport and do not show this redirect selector.
  The proxy route uses the single registered callback
  `https://app.acheflow.com/callback/`, independent of the studio address.
  It requires publishing the matching static link pages. The UI discloses that
  the selected proxy page can read the callback in the browser. The page shows
  the original studio's HTTPS origin and requires **Return to browser studio**
  before sending the callback to that window. Exact source/origin and a random
  transport challenge fence delivery; the guest still verifies OAuth state,
  application identity and scopes before committing credentials. No arbitrary
  return URL is followed. Cancellation, window disconnection and expiry require
  starting again; there is no silent fallback to a native app or another route.
  The relay uses no token storage, backend, analytics or third-party scripts.
  Implicit access tokens in URL fragments are not included in HTTP requests;
  authorization codes or other query parameters can reach hosting infrastructure.
  The site operator can change served JavaScript, so hosted sign-in requires trust
  in that operator and hosting account. The page clears its URL immediately. The
  browser relay discards its callback after delivery, navigation or five minutes;
  installed-app callbacks also require a live, unexpired local pending request.
  The direct route uses the bundled `oauth-callback.html` alongside the current
  studio, preserving reverse-proxy base paths. For example,
  `https://studio.example/` uses `https://studio.example/oauth-callback.html`.
  Register that exact HTTPS address in your provider application. The existing
  studio server serves it; no separate callback service is required.
  Browser provider API traffic remains direct in both modes. CORS failures do not
  fall back to server-held credentials. A Twitch `redirect_mismatch` means the
  requested callback does not match the application's registration; previous
  consent alone does not establish a successful new sign-in.
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
**Send chat**. If the channel also has a saved rule that limits it to reading,
enable **Send chat** for the account under **Extensions → Server → Channel permissions**.
Platform authorization is still required. Native VK polls remain unavailable.
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


Under **Settings → General → Callback proxy**, configure the **Client callback
proxy URL** for this device's workspace or **Server callback proxy URL** for the
server's accounts. An empty value uses `https://app.acheflow.com`. Custom values
are HTTPS origins without paths, query strings, fragments or embedded credentials.
Publish the same web-client and callback bundle at that origin and register its
`/callback/` URI with the provider. Server changes require
`extensions_configure` and are revision-checked and persisted on the server;
client preferences never enter the server configuration or other devices.
Changing a proxy does not move tokens or change the server connection.

If a server card reports **Credential storage unavailable**, unlock the server's
credential store, open **Account & settings** and choose **Save settings** to
retry the saved connection. Temporary storage failures do not imply revoked
authorization and must not force a new provider sign-in.

The callback router itself is provider-neutral. `/callback/` and its contextual
subpaths, such as `/callback/<context>/…`, also accept non-OAuth responses.
The path does not select a workspace or grant access. A callback field can carry
`acheflow-callback:` followed by a JSON envelope with `v: 1`, `target: native|browser`,
and a one-time `nonce`; native delivery additionally names the declared `/fwd/...`
route. OAuth uses `state` as that carrier. No token, workspace selection or
arbitrary return URL belongs in the envelope. The router restores the original
nonce in the carrier before delivery. The waiting client owns the actual
workspace/instance mapping and rejects stale, replayed or unmatched callbacks.

## Storage and lifetime

On macOS, committed native credentials use Keychain. One record contains the
accounts for a verified server identity and workspace, with isolated slots for each
extension and public application ID. One account is active per slot. Disconnect
removes only that slot; the last disconnect removes the empty workspace record.
Disabling the client part removes its active session without deleting the saved account. A locked or
unavailable keychain produces an error; there is no plaintext-file fallback. Other
native platforms need their own OS vault adapter before durable use.

Existing per-extension records migrate when that account is next opened or saved.
The new workspace record is saved before the old record is removed; interrupted
cleanup is retried without restoring an older token over the new one. Unused old
accounts remain in their original records until accessed. Native processes serialize
read/modify/write operations with an OS file lock containing no credentials. URL
slots migrate to the verified server namespace after a recoverable destination
exists; conflicting accounts are preserved. A process-memory cache and shared
revision marker avoid repeated OS reads while invalidating records before writes.
Keychain prompt counts depend on signing and access policy, separately from record
counts. Device-to-server login has its own single container and session lifetime.
Storage is bounded to 64 slots and 4 MiB per workspace, with the existing 256-KiB
per-account limit. An unsuccessful save reports an error and preserves saved data.

Consolidating local Keychain records does not transfer accounts to the server.
The explicit [trusted-server connection](#connect-a-trusted-server-account) below
uses its own server/workspace namespace.

Browser credentials live only in memory. They are never written to localStorage,
IndexedDB or workspace export. Personal browser credentials never reach the server. Server authorization uses
its own temporary session. Disable/workspace/server changes dispose
the local instance and discard browser credentials. Late results cannot restore it.

## Connect a trusted server account

Open the shared workspace, then **Extensions → Server → Configure**. Select the
needed read functions and sign in to the server's own account. Application setup
and OAuth remain extension-owned; native callbacks terminate on the authorizing
device, while browser flows use that page's origin. Select the exact shared channel
ID, review the trust notice and explicitly save the authorization to the server.
The trusted server can access its stored access/refresh tokens and application
secrets required for renewal. Use trusted HTTPS for remote servers; loopback HTTP
is allowed for development. Your Client account remains separate.

The server revalidates identity and current rights through its installed guest,
then saves a fresh isolated slot. Validation is bounded to thirty seconds, eight
HTTPS effects and two concurrent checks, with no collector during validation.
The workspace ID and expected revision protect concurrent changes. Configuration
rights permit reading/saving nonsecret settings; execution rights are additionally
required for account authorization, module execution, starting/stopping collection
and disconnection. No scene/media/access-administration grant is required merely
to manage the server integration.

The saved collector receives chat/audience under read grants, resumes after server
restart, and runs independently of video output. Saving never starts a stream.
Replacement cancels the old collector before using the new binding; failed storage
preserves the previous usable binding. The temporary authorization session is
cleared after saving. **Save server settings** updates public parameters; changing
the application Client ID requires reauthorization. **Stop collecting** preserves
the account. **Disconnect server account** stops it and removes its credential slot.
macOS has the OS vault adapter; other native server platforms fail closed until
their adapter is implemented.

## Personal accounts on the trusted server

This is a migration-only section. Earlier server-held personal copies remain
listable/deletable by their authenticated owner in **Client → Old personal accounts
stored on this server**. Reauthorize in Client on each device before deleting a
copy. The UI never downloads secrets or automatically removes the sole copy.
A locked server vault retains cleanup metadata so deletion can be retried after
unlocking. Old personal upload/send endpoints return 410 and cannot execute actions.
Linking devices shares Acheflow identity and permissions, never platform accounts.

## Execution boundary and verification

The server is the sole shared-chat collector. Opening a client or signing in to
its sender never registers another read subscription. The composer shows this
device's account and one explicitly selected channel. Sending validates current
Acheflow rights, the committed platform account and target metadata before one
direct platform write. Navigation does not retry an in-flight send; unknown delivery
is terminal. Changing account/workspace, revocation or disconnection fences the
request. There is no server-account fallback. Future moderation is not implemented.

Native and browser use the same action-only WASM path with bounded HTTPS. Browser
CORS/OAuth restrictions may prevent a platform request; the UI reports failure and
never uploads personal credentials to work around it. Use the native client where
needed. Real-WASM regression tests cover all three provider sender paths, loopback
writes, revocation, callback fencing and uncertain delivery. Separate live consent,
renewal, physical-phone keyboard and signed-build Keychain checks still require
the intended applications/accounts and devices.

Sources: [Twitch OAuth](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/),
[Twitch app registration](https://dev.twitch.tv/docs/authentication/register-app/),
[Twitch scopes](https://dev.twitch.tv/docs/authentication/scopes/),
[Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[Google browser token model](https://developers.google.com/identity/oauth2/web/guides/use-token-model),
[Google browser client setup](https://developers.google.com/identity/oauth2/web/guides/get-google-api-clientid),
[VK Video Live authorization](https://dev.live.vkvideo.ru/docs/main/authorization),
[VK chat methods](https://dev.live.vkvideo.ru/docs/method/chat).
