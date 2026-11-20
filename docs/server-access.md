# Server device access

The studio HTTP API requires an authenticated device, including on loopback.
Static UI assets, `/healthz` and minimal `/api/server/info` discovery remain public.
Acheflow device sessions are separate from provider accounts and OAuth scopes.
After authentication, clients follow the server workspace. Devices without full
Apply grants receive a display projection with keys, transport URLs, extension
configuration and capability policy removed; scene/source details also require
studio, scene-control or interactive-view rights.

The server separates users and devices. Personal provider credentials stay on
each originating device; sending requires both current Acheflow `chat_send` and
platform permission. Shared chat reading grants neither sending nor future
moderation. The server collector has its own independently authorized service
account. See [extension ownership](extensions.md#connect-chat-and-send).

## Connect devices

The desktop that launches its own server reads its protected local owner authority
and connects as **Local server owner**. On macOS this authority is retained in the
OS vault for reconnection to a detached server. Merely discovering an unrelated
server on localhost grants neither authority nor process ownership.

In **Settings → Device access**, use **Your profile & devices** to change your
display name or **Link my other device**. A device link attaches another device to
your user with the issuing device’s current permissions. Use it only on your own
devices: linked devices share an Acheflow identity. Platform accounts remain device-local,
and linking never combines device permissions.

To add someone else, use **Invite a participant**, choose a role and select
**Create invitation**. This always creates a separate user, even if names match.
Viewer, Chat, Scene operator, Operator and Administrator presets cover common
needs. **Customize permissions** exposes individual grants when needed.
Share the code privately. On the other desktop or browser, open the same server,
enter a device name and the code in that section, and select **Connect device**.
Codes expire after ten minutes and work once. Pairing accepts ten attempts per
peer IP per fifteen minutes; reverse-proxy clients share the proxy's peer budget.
Forwarded headers do not grant trust.

Remote browser clients need trusted HTTPS, for example a Tailscale Serve HTTPS
address. Native authorization accepts HTTPS or loopback HTTP and does not follow
redirects or use an HTTP proxy. Browser sessions use a host-scoped
`HttpOnly; Secure; SameSite=Strict` cookie with a thirty-day lifetime. Same-origin
pages restore it after reload. Static clients on a different explicitly allowed
UI origin keep a Bearer token in memory and need a new invitation after reload.
No token is put in a URL, workspace export, localStorage or IndexedDB. macOS native
sessions use one separate OS-vault container with endpoint-scoped entries, inaccessible
through provider workspace storage; native platforms
without a vault adapter retain device tokens in memory only.
Native device-session reads and writes run off the UI executor so an OS-vault
permission prompt does not block the studio interface.

For a standalone headless server, obtain `owner_key` locally from its private
access file: settings `server.json` use `server.access.json`. Use the setup key in
the pairing form to create an administrator device. Subsequent devices should use
participant invitations or explicit own-device links. The setup key is never printed to startup logs or served over HTTP.
The file contains recovery authority and session/invitation hashes; protect it and
its backups as administrator credentials. On Unix it must be a regular private
file (0600). On Windows keep its directory private through inherited ACLs. Startup
fails closed on corruption or unsafe Unix permissions rather than resetting access.

## Permissions

Grants are independent. **Manage device access** does not imply other rights;
administrators can invite/revoke only within their own permissions. Revocation
invalidates issued invitations and withdraws the device's Studio executor
registrations. New requests then fail; provider writes already in flight cannot
be undone. A device may disconnect itself. Local recovery authority cannot be
revoked remotely. Any authenticated user can list their own active devices and
revoke a linked device, including one with more permissions; this does not grant
the caller any of those permissions. Changing a display name never changes
ownership or permissions.

Access-file version 1 migrates atomically to version 2 before startup completes.
The owner gets a stable user identity, and each legacy device gets a separate
identity; names and administrator rights do not prove shared ownership. Existing
tokens, expiry, grants and outstanding participant invitations are retained.
User identities remain after device revocation for stable future account ownership.
The store limits retained users to 1,024 and active devices to 128.

| Permission | Operations |
| --- | --- |
| View studio | Counters, image previews, public scenes and visual documents |
| View interactive | Polls, rules, donations and interactive state |
| Read chat / Send chat | Independent merged-chat read and typed send rights |
| Control scenes | Existing spatial-scene commands |
| Start / stop media | Stop/restart outputs and refresh encoder capabilities |
| Apply workspace configuration | Full Apply/Start, together with scene/media/extension configure/run grants |
| Control interactive | Create/close polls; scene-driven rules/actions also need scene control |
| Configure extensions / Run extensions | Separate public configuration and enable/disable/executor rights |
| Upload extensions / Install extensions | Independent staged ZIP upload and activation grants; execution requires separate approval |
| Manage device access | Invitations, revocation, server settings and diagnostics |

Full Apply/Start contains sources, outputs and extensions, so it requires every
corresponding management grant. A narrowly granted scene operator uses scene
commands rather than replacing the entire configuration. Generic guest panel
events require configure/run, chat-send, scene and interactive management because
panels can invoke their declared effects.

Read and command responses are filtered on the server. Viewers do not receive
extension settings/account details, raw chat or private errors without the
corresponding rights. Studio preview includes rendered overlays: chat-read denial
cannot hide text already visible in video pixels. Provider tokens and stream keys
are never included in device/session responses.

The server persists and synchronizes its shared workspace. Explicit server-owned
integrations execute once on the trusted server; existing local accounts remain
on clients unless an administrator explicitly uses the
[trusted-server account connection](extension-authorization.md#connect-a-trusted-server-account).

## HTTP and MCP

Native/API clients send `Authorization: Bearer <DEVICE_TOKEN>`. Mutations also need
`X-Acheflow-Request: 1`; JSON endpoints need `Content-Type: application/json`.
Browser origins must match the public HTTPS host or be explicitly configured with
`ACHEFLOW_UI_ORIGINS`. Loopback HTTP origins are permitted for local development.
Preserve `Host` through a reverse proxy, or configure the actual UI origin
explicitly. `X-Forwarded-*` never authorizes requests. CORS permits the explicit
request/auth headers for allowed origins, without cross-origin cookie credentials.
Access responses are non-cacheable. Unknown API routes are denied by default.

| Endpoint | Operation |
| --- | --- |
| `POST /api/access/pair` | Exchange `{code, name}` for a session and secure cookie |
| `GET /api/access/session` | Read user/device/server identity, expiry and exact grants |
| `PUT /api/access/profile` | Update the caller’s display name with `{name}` |
| `POST /api/access/link-invitations` | Create `{permissions: [...]}` for the caller’s own user; grants must be a nonempty subset of the issuing device |
| `GET /api/access/my-devices` | List the caller’s active linked devices without tokens |
| `POST /api/access/invitations` | Administrator: invite a new participant with `{permissions: [...]}`; return the code once |
| `GET /api/access/devices` | List paired devices and grants without tokens |
| `DELETE /api/access/devices/{id}` | Revoke a device and its outstanding invitations |
| `GET /api/access/personal-accounts/{workspace}` | List only the authenticated user’s personal account metadata |
| `POST /api/access/personal-accounts/{workspace}/{provider}` | Retired: returns 410; reauthorize locally on each device |
| `DELETE /api/access/personal-accounts/{workspace}/{provider}` | Disconnect the caller’s binding with `{expected_version}`; report credential cleanup separately |
| `POST /api/access/server-accounts/{provider}` | Revalidate and connect a workspace account; ExtensionsConfigure + ExtensionsRun required |
| `DELETE /api/access/server-accounts/{provider}` | Remove server credentials after stopping collection; ExtensionsConfigure + ExtensionsRun required |
| `POST /api/access/personal-send/{workspace}/{provider}/{channel}` | Retired: 410, with no external write |
| `POST /api/access/actions/authorize` | ChatSend: validate current workspace and connected provider/channel; return short-lived authority metadata, never credentials |
| `GET /api/extensions/server-providers` | ExtensionsConfigure: list public server manifests |
| `GET /api/extensions/server-providers/{provider}/module` | Configure + Run: load approved server authorizer WASM, with no saved secret state |
| `GET /api/access/server-accounts/{provider}` | Configure: read that provider’s public binding/settings and revision, without output/ingest keys |
| `PUT /api/access/server-accounts/{provider}` | Configure: save public settings; changing enabled state additionally requires Run, with expected workspace revision |
| `GET /api/extensions/packages` | Upload, Install or Run: inspect package metadata |
| `POST /api/extensions/packages/stage` | Upload: bounded ZIP validation and staged review |
| `POST /api/extensions/packages/install` | Install: atomically activate reviewed package with execution disabled |
| `POST /api/extensions/packages/allow` | Run: approve execution of the exact installed digest |
| `POST /api/access/logout` | Revoke the caller's session and clear its cookie |

Native pairing adds `X-Acheflow-Client: native` to receive its new token in the
response body. This controls delivery format, not authority. Permission identifiers
are shared API snake-case values, for example `studio_view`, `chat_read` and
`access_manage`. Invalid/revoked sessions return 401, denied grants 403 and rate
limits 429.

The separate loopback MCP listener retains its explicitly configured operator,
reader and workspace-pinned executor credentials. Device tokens do not become MCP
operator credentials. Reader snapshots use the same read projection as combined
studio/interactive/chat viewers. Executors cannot call operator tools. See
[executor configuration](client-executor.md).

Cookie behavior follows [MDN's Set-Cookie reference](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie).
