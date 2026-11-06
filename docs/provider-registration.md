# Provider application setup and live registration (DL-12)

Status: registration started on 2026-11-05. Google project `acheflow` exists and
YouTube Data API v3 is enabled. VK Video Live application Acheflow is active.
Native Twitch sign-in, validation and restoration after restart are verified for
`strelkadev` with `user:read:chat`. Native VK sign-in, validation and restoration
after restart are verified; its validation did not expose an account name.
Google token acceptance and broader functions remain unverified. No quota request
has been submitted. Application IDs and secrets are supplied by each user;
embedded defaults are removed.

## Owners and registration order

| Order | Provider | Intended owner supplied by maintainer | Registration status |
| --- | --- | --- | --- |
| 1 | YouTube / Google Cloud | Google identity confirmed by maintainer as owner of `achedev` | Project `acheflow`, number `74963158585`; API enabled; OAuth configured; Acheflow Desktop and Acheflow Web clients created |
| 2 | VK Video Live | `achedev`; maintainer completed portal login | Acheflow active; ID retained only in the local test workspace |
| 3 | Twitch | `strelkadev`, displayed as `StrelkaDev` in console | Acheflow Desktop created; ID retained only in the local test workspace |

Choose a recognizable, available application name. The maintainer fixtures use
Acheflow / Acheflow Desktop.
Confirm the signed-in account before creating resources. Browser access is now
available. Login and any 2FA must be completed in the browser; do not paste
credentials into chat, task files or commits.

## Required Connect experience and target links

Users create their own provider application and fill its requisites above Connect.
Connect remains disabled until required fields are present. Application IDs are
public workspace settings; application secrets stay in local credential storage.
No maintainer IDs or secrets are automatically inserted. After sign-in, Disconnect
is visible in the card without expanding advanced token entry.

| Provider / target | Required fields | Registration |
| --- | --- | --- |
| YouTube native | Desktop Client ID and local application secret | Google Cloud: enable YouTube Data API v3, configure consent/test users, create Desktop app |
| YouTube browser | Web Client ID | Google Cloud: create Web application and allow the exact JavaScript origin |
| Twitch | Client ID | Twitch developer console: Confidential app for the implicit grant, exact callback |
| VK Video Live | Client ID | VK Video Live DevAPI application with the exact callback; no VK ID token substitution |

Register an application/project under the account you trust. YouTube quota belongs
to the Cloud project; multiple Client IDs in one project do not provide separate
quota. Maintainer registration details below are historical live-test evidence.

The maintainer selected `app.acheflow.com` for deep links, keeping the agreed
`/fwd/<extension_id>/<action>/...` route. The host supplies the trusted link base
for that extension; the extension declares a suffix such as `oauth/callback`.
The complete Twitch target is
`https://app.acheflow.com/fwd/twitch/oauth/callback`; VK uses `vklive` as its ID.
Provider consoles register the complete URL, not a suffix or a literal
`/extension/oauth/callback` endpoint. Both extension identity and pending `state`
must match before a callback is consumed.

The HTTPS return page is deployed; native Twitch/VK consent and installed custom-scheme
delivery are verified. Google token acceptance remains pending. Google Desktop
keeps its supported dynamic loopback/S256 PKCE flow behind the same Connect
action; Google browser uses Identity Services. Do not force a Web client/central
secret exchange merely to share a hosted callback. Browser callbacks must not
accidentally activate a native app. See the
[revised desktop acceptance criteria](../tasks/plan-desktop-links.md).
VK's registered native callback is now
`https://app.acheflow.com/fwd/vklive/oauth/callback`; the fixed loopback entry was
replaced. Live consent and custom-scheme delivery are verified; signed HTTPS
universal-link delivery remains pending.

## Live registration checkpoint — 2026-11-05

The maintainer approved enabling YouTube Data API v3 and accepting its displayed
terms. Google OAuth branding is prepared as Acheflow, External/testing audience,
with the current Google account address temporarily selected for user support
because the dropdown did not offer `alex@strelka.dev`. The developer contact is
`alex@strelka.dev`. Created the approved **Acheflow Desktop** client; its public ID
is retained in the local test workspace.
Created the separately approved **Acheflow Web** client, with
`https://app.acheflow.com` as its only authorized JavaScript origin and no redirect
URIs. No generated client secret was saved in the repository or bundled configuration.

The maintainer approved creating VK's application with the native development
callback `http://127.0.0.1:21941/oauth/callback`; the portal confirms Active.
The generated secret was not copied into project files or configuration. The
maintainer approved replacing the development callback with the HTTPS address
above; it was saved and verified after reopening settings. No webhook or browser
redirect was added. App creation is not live OAuth acceptance.

Twitch rejected the globally occupied name Acheflow. Created **Acheflow Desktop**
under StrelkaDev with the approved Broadcaster Suite category, Confidential type,
and `https://app.acheflow.com/fwd/twitch/oauth/callback`. The saved settings confirm
the client ID retained in the local test workspace. No client secret was generated
for the guest's implicit flow.

Deployed the generated static return page to Cloudflare Pages project
`acheflow-app` (`acheflow-app.pages.dev`). The maintainer added the external REG.RU
CNAME `app` → `acheflow-app.pages.dev`; Cloudflare confirms `app.acheflow.com`
Active. Public HTTPS checks for both provider callback paths return 200 with the
original URL intact. The Twitch callback response matches the generated page and
includes no-store/no-referrer and the restrictive CSP. All three script/style
assets return 200. The Pages rewrite targets extensionless `/forward`; targeting
`/forward.html` caused a canonicalizing redirect and was corrected before this
checkpoint. The initial browser-only check rendered the enabled Open Acheflow button, cleared
the callback from browser history, and reports no console warnings/errors. It
does not activate an installed handler. No Apple association file is deployed
without a real signing identity; the local signing-identity query found none.
The maintainer confirmed that no Apple signing certificate is available yet.
For now, the hosted page offers an explicit custom-scheme Open action; automatic
HTTPS universal-link delivery is deferred. This does not prevent testing OAuth
with the custom-scheme fallback, and does not change provider callback URLs.

Native code now allocates an independent temporary loopback port for Google;
Twitch/VK guests select the hosted return by declaring only `oauth/callback`.
The host injects their installation-derived link base and validates mailbox
delivery. Application fields and local secret inputs are
extension-declared; no application defaults are supplied. Signed HTTPS association,
verification of Google's configured native flow, browser deployment
and any quota requests remain outstanding.

An isolated native workspace with the registered public IDs and read-only
functions exposed a startup session-cache race: the initial server cleanup could
delete sessions after their panels initialized. A two-provider UI regression
reproduced it. Session cache keys now include server identity and selective cleanup
retains newly initialized sessions. Startup and server-change cases pass, as does
the updated 269-test workspace suite (11 existing ignored). Native Connect now
opens Twitch in system Safari and waits for its callback. The maintainer completed
the Twitch login/consent in Safari. After installing the test bundle in the user's
Applications directory and registering its helper, the full HTTPS return → custom
scheme → native mailbox → real Twitch validation path succeeded. The panel showed
`Authorized locally as strelkadev`, and restored that state after closing/reopening
the app through Keychain plus provider validation. This verifies the account flow,
not an executor connection, chat subscription, send or poll operation.

Live testing exposed that Twitch's documented implicit callback omits `expires_in`.
The SDK now defers missing callback lifetime to a declared validation expiry field
for implicit flows; missing/zero/invalid validation lifetime cannot commit secrets.
PKCE token responses and providers without such validation still require lifetime
in the token response. SDK and real-WASM regressions cover the actual fragment form.
The registered helper in the system temporary directory was not selected by
LaunchServices; the same signed bundle under Applications was selected. No
Developer ID certificate or universal-link association was used for this
custom-scheme check.

### Native continuation — 2026-11-06

VK's real callback completed and provider validation accepted the token. The UI
showed `Authorized locally`, including after restarting with the recovered test
workspace. No account nickname was returned to the panel, so ownership of `achedev`
has not been independently confirmed from the validation response.

The test app's temporary workspace was cleaned between sessions. Its workspace
identity was recovered from Keychain record attributes without reading secret
values; the isolated fixture now lives in ignored `target/dl12-live/`. The installed
test app and helper use that stable location. This is local test configuration.
An initial Twitch validation attempt failed, but after the next restart the same
saved account validated as `strelkadev`; no new login was needed. Do not infer
token expiration from the earlier generic failure.

Google initially rejected the selected AChe Dev brand with `403 access_denied`
because the External/Testing app had no test users. The owning Google identity is
now allowlisted. A fresh native Connect selected AChe Dev, passed the tester check
and reached consent for `youtube.readonly`. The maintainer completed consent and
the ephemeral loopback received the code. Exchange then failed with HTTP 400
`invalid_request`. A separate request with a deliberately invalid diagnostic code
confirmed that this Desktop client first rejects missing `client_secret`. The
native setup now requires that local field before launching a browser, and the
real-WASM test verifies the secret handle in the PKCE exchange. No user token or
application secret has been embedded. Live token acceptance remains pending.

The console masks the existing secret and explicitly disallows viewing or
downloading it again. The maintainer then chose explicit user-owned application fields instead of
embedded requisites. No additional secret was created; existing secrets have not
been changed or removed. Google live acceptance requires entering a valid Desktop
secret through the local field.

The Google Console Audience page still shows Testing and disables Publish app,
explicitly requiring completion of Branding. The application home page, privacy
policy and terms links are empty; `acheflow.com` is already an authorized domain.
Public publication and Google verification are separate from enabling the API or
adding a tester. No publication or verification submission has been performed.

## Registration parameters

Copy active addresses from **Application setup** in each extension card. Browser
origins and callback paths depend on the actual deployment; do not substitute an
unverified production domain. See [local authorization](extension-authorization.md).

### YouTube

1. In the [Google Cloud console](https://console.cloud.google.com/), confirm the
   Google identity owning the `achedev` channel. Select or create the Acheflow
   project and enable YouTube Data API v3. Record its project ID and number.
2. Configure consent branding, audience, support contact, verified domains and
   required privacy/homepage links. Keep test users while testing; record any
   required OAuth verification before public rollout.
3. Create a **Desktop app** client for native Acheflow and a **Web application**
   client for the browser. Native uses authorization code + S256 PKCE and the
   dynamically allocated loopback callback shown during sign-in. Browser
   uses Google Identity Services' token flow and exact authorized JavaScript
   origins. It does not use a hosted redirect or application secret.
4. Enter the Desktop application's secret in the local Application secret field.
   The registered Desktop client returns `client_secret is missing` without it,
   despite the optional wording in Google's general parameter documentation.
   Native uses S256 PKCE and secret handles; no credential value is bundled.
   A real-WASM regression verifies the setup gate and exchange parameters.
5. Test `youtube.readonly` for reading and `youtube.force-ssl` for sending only
   when that function is enabled. Record consent, refresh and revoke results.

References: [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[browser setup](https://developers.google.com/identity/oauth2/web/guides/get-google-api-clientid).

### VK Video Live

Use the [DevAPI portal](https://dev.live.vkvideo.ru/) under `achedev`. Register
Acheflow with the exact native and deployed browser callback URIs. The existing
`vklive` guest declares implicit `response_type=token`, no application secret,
and read-only chat/audience functions without privileged scopes. Sending and polls
are not implemented. VK ID sign-in does not make a VK ID token a DevAPI token.

These parameters come from the current extension manifest. The official
[authorization page](https://dev.live.vkvideo.ru/docs/main/authorization) was not
accessible to the documentation fetch on 2026-11-05. Confirm supported flows,
approval requirements and limits in the live portal before submitting or claiming
compatibility. No numeric quota or increase procedure is assumed.

### Twitch

Use the [developer console](https://dev.twitch.tv/console/apps) under `strelkadev`.
Verified email and 2FA are prerequisites. Register Acheflow with the exact callback
URIs. The current guest uses implicit grant and requires a compatible
**Confidential** application type; Twitch **Public** clients support device flow,
which this guest does not offer. No client secret is used by this implicit flow.
Select only the functions to test: `user:read:chat`, `user:write:chat` and
`channel:manage:polls` map to separate guest settings.

References: [registration](https://dev.twitch.tv/docs/authentication/register-app/),
[OAuth flows](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/).

## Quota evidence before requesting increases

Record a dated observation period, actual active installations/channels, session
hours, calls by endpoint, reconnect counts, errors and peak demand. Keep tokens,
chat contents and private user identifiers out of shared evidence. Record the
currently assigned limits from each project's console rather than estimating
them from a generic default.

For YouTube, attach the source/project URL, release version, feature demonstration,
data-handling description, measured usage and a calculation of requested daily
units using current endpoint costs, including streaming reconnects and sends.
Increased quota requires the [YouTube audit and quota extension process](https://developers.google.com/youtube/data/guides/quota_and_compliance_audits).
OAuth consent verification and this quota audit are separate rollout records.
Extra OAuth Client IDs in one project do not create independent quota. Do not
split one Acheflow use case across projects to bypass limits: the
[developer policy guide](https://developers.google.com/youtube/terms/developer-policies-guide)
explicitly prohibits quota sharding. This also qualifies the project-quota note
in the original DL-12 plan.

For Twitch, record API rate-limit headers and HTTP 429 occurrences; Helix limits,
EventSub constraints and chat limits need separate evidence. Follow the
[API rate-limit guide](https://dev.twitch.tv/docs/api/guide/#twitch-rate-limits)
and current endpoint documentation. Confirm a relevant request route with the
provider if measurements justify it. For VK Video Live, first establish assigned
limits and the support/request route in the portal. No free increase is promised.

## Evidence required for a configured application

For each provider and native/browser flow, record:

- Verified owner, app/project identifier, public Client ID and registered addresses.
- Consent/audit/approval state, date and evidence reference; mark inapplicable
  approval explicitly instead of inferring it from app creation.
- Assigned quota, measurement period, requested amount, request reference and
  provider decision. Leave requests pending until there is actual evidence.
- Live sign-in, sender identity, read/send permissions, expiry/renewal where
  supported, cancellation, disconnect/revocation and restart results.
- Validation with another user-owned application: its fields and separate local
  credential slot still work. Verify native and browser separately.

Store credential values only through the local credential UI. Shared records may
contain public app identifiers and redacted evidence, never user tokens, refresh
tokens or confidential secrets. Fields, instructions and provider behavior belong
to the owning extension; shared UI/server code remains provider-neutral.
DL-12 stays unchecked until the applicable registration, quota and live checks
are recorded. Registration preparation alone does not satisfy acceptance.


### Final form verification — 2026-11-06

271 workspace tests passed (11 existing ignored), 16 guest tests passed, and native
and browser-WASM Clippy passed with warnings denied. The installed macOS test app
shows the public Client ID and required YouTube Desktop secret above Connect; an
empty required field disables submission, including keyboard activation. Twitch's
visible Disconnect cleared its account and a fresh live hosted callback restored
`Authorized locally as strelkadev`, with Disconnect visible again. VK restored its
independent saved authorization. Google still needs a valid local Desktop secret
for a live exchange; Testing/Branding has not been published or submitted.
