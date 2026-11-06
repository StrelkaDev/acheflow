# Installed application forwarding

The forwarding address contract is implemented by
`acheflow_extension_api::forwarding::ForwardLink::parse_for`. Manifest-declared
delivery is available for live client sessions and server guests. Unix desktop
clients also provide a local activation receiver. Signed OS/domain association
and hosted fallback pages are subsequent tasks; parsing an address alone does not
authorize an extension action.

## Addresses

| Origin | Recipient |
| --- | --- |
| `https://app.acheflow.com` | Installed client |
| `https://server.acheflow.com` | Installed server |

Both use `/fwd/<extension_id>/<action>/...`. For example:

```text
https://app.acheflow.com/fwd/example/oauth/callback?code=opaque&state=opaque
```

`acheflow.com` remains the product website. `app.acheflow.com` serves the browser
client and reserves `/fwd/*` for installed-client links; ordinary browser routes
are not app links. The former `client.acheflow.com` origin is rejected (no release
requires compatibility). The forwarding origin is not a user's server endpoint.
Each receiver supplies its expected recipient to the parser; server/client links
cannot be substituted for each other. Canonical HTTPS authorities are required,
without explicit ports, user information or alternate host spellings.

The extension ID is the exact, case-sensitive manifest ID, validated by the same
rules as installation discovery. It is not a display name or a directory name.
The action is one nonempty ID segment of up to 64 ASCII letters, digits, underscores
or hyphens. Remaining path, query and fragment are opaque provider payloads.
URLs are bounded to 16 KiB. Noncanonical paths, malformed percent escapes, path
traversal and encoded path separators are rejected. ForwardLink's Debug output
omits the tail, query and fragment; applications must also avoid logging raw URLs.

## Installation ambiguity

Within each recipient's catalog, all installations sharing a valid manifest ID
are disabled. A third occurrence cannot restore an ambiguous ID. Errors identify
the conflicting directory paths or bundled installation slots. Unrelated valid
providers remain available, subject to the existing catalog capacity.

Directory scanning fails closed when the 64-entry scan limit is exceeded or entries
cannot all be read, rather than accepting a partial catalog with hidden duplicates.
Rejected duplicates cannot contribute browser authorization modules to a retained
installation.

## Declared delivery

Manifests optionally declare up to 16 unique actions. Existing manifests default to
no forwarding support. The provider chooses the action names and callback flow:

```json
"forwarding": [{"id":"oauth","mode":"callback"}]
```

`event` actions require a live instance. `callback` actions are client-only and also
require a pending grant. A client guest emits the capability-checked `forward_wait`
effect with its action, correlation field, host-issued crypto nonce and timeout
(at most five minutes). The host checks the correlation before staging credentials
and before guest delivery. Wrong correlations leave the legitimate grant pending;
accepted callbacks consume it once. Query and fragment fields are decoded into a
bounded object; duplicate names are rejected rather than choosing one value.

`LocalSession::forward` requires its bound workspace and current instance identity.
Starting/restarting rotates that identity; stop, cancellation/discard and fatal
guest errors revoke pending callbacks. The host captures the manifest's secret
fields behind opaque handles, then delivers `local_forward` with `action`, `tail`
and redacted `payload`. No raw URL/query/fragment is sent to the guest. The SDK's
authorization helper interprets its own `oauth/callback` action and still validates
provider state, credentials and permissions before committing an account.

The forwarded envelope includes the host-validated `mode`. Ordinary events cannot
carry declared secret fields and never modify staged authorization credentials.
Stopped or failed local sessions reject late results until explicitly restarted.

Server guests accept only declared `event` actions, delivered as `forwarded` with
the same action/tail/payload shape. The existing control API provides
`forward_extension` with `url` and expected `instance_id`, under normal workspace,
revision and request-dedup checks. Snapshots expose each live extension's instance
ID. Delivery uses a private, 32-entry queue owned by that instance; restarting drops
the old queue. Links are not broadcast to other extensions. Server manifests cannot
request callback grants or provider credential storage.

Native receivers must select an installed, enabled session from the active
workspace before calling these APIs; they must not create one merely because a
link names it. Provider flows, scopes, token handling and callback interpretation
remain under `extensions/`. The host supplies `environment.link` as
`https://app.acheflow.com/fwd/<installed_manifest_id>`, overriding caller values.
The guest SDK's optional `authentication.native_callback` is only a relative
action/tail, e.g. `oauth/callback`; it must resolve to this installation's declared
callback action without response parameters. Native Twitch/VK use this link;
Google Desktop retains code/S256 PKCE and an independently allocated loopback
port. Browser callbacks retain their browser origin/path. The browser effect
carries the selected `callback_uri`; native driving waits on the prepared loopback
listener or the checked app-link mailbox, with a five-minute deadline and
cancellation. Portal setup, hosting and signed app association still require
live verification before official defaults are enabled.

## Desktop client activation

On Unix, `acheflow-ui` accepts `--show` (also the default), `--forward <URL>`,
a single forwarding URL argument, or `--forward-stdin`. `--help` and `--version` do not
start a GUI or activation listener. OS adapters should use `--forward-stdin`,
write the exact URL bytes, then close stdin. This keeps callback credentials out
of process arguments. Input is bounded to 16 KiB; trailing whitespace is rejected.

`acheflow-launcher` elects one client per user/runtime directory using a held file
lock. A later invocation hands the request to that client and exits. The receiver
uses a mode-0600 Unix socket in an owned mode-0700 directory; it opens no TCP port
and stores no callback payload on disk. The default directory is
`<system temporary directory>/acheflow-<uid>`. `ACHEFLOW_RUNTIME_DIR` can select a
different private directory for isolated development profiles. Client and server
use separate locks and socket names. Stale owned sockets are recovered only while
holding the election lock; regular files and symlinks are rejected.

Handoffs use bounded frames, queues and receipt deadlines. A successful reply means
the live local session accepted the event, not that a provider authorized an
account or completed a later HTTP operation. If the receiver cannot confirm the
result, the sender reports an unknown outcome and does not automatically resend.
Queued requests are checked again for expiry before delivery.

The desktop UI restores/focuses its window and selects Extensions for forwarded
requests. Missing/conflicting installations, disabled extensions and unavailable
sessions produce explicit errors without displaying the callback URL. Delivery is
bound to the workspace, configuration generation and current session instance.
An incoming callback can complete a pending native browser operation; invalid
callbacks leave the legitimate operation pending. Browser builds retain their
separate navigation and authorization flow. Windows activation is not implemented;
ordinary desktop launch remains available there.

## macOS development bundle

The AppKit adapter in `native/macos/ClientLauncher.swift` receives URL-open and
browsing-user-activity events and launches the fixed bundled Rust client with stdin
handoff. It contains no provider logic. Build a local bundle with:

```sh
cargo build -p acheflow-ui --bin acheflow-ui
python3 native/macos/build-dev-app.py \
  --output target/AcheflowDev.app \
  --ui-bin target/debug/acheflow-ui
```

Choose a new output directory. The main bundle owns the Rust GUI executable;
`Contents/Helpers/AcheflowLink.app` is the URL-event helper. For isolated manual
testing, launch the helper with `ACHEFLOW_RUNTIME_DIR` and
`ACHEFLOW_WORKSPACE_FILE` pointing to temporary profile paths. Opening a URL with
an explicitly selected helper exercises its URL-event adapter; the development
bundle does not register itself as the default HTTPS handler.

This bundle is unsigned and does not establish HTTPS domain association. The
association/site artifacts and handler installation instructions are described in
[native-links-installation.md](native-links-installation.md). Production signing
remains external rollout work; complete local desktop packages are described in
[desktop-installation.md](desktop-installation.md). The IPC/process suite and native visual smoke check remain pending
for this slice; compilation alone does not verify cold/warm OS activation.

## Native server activation

Server reception is explicitly enabled on Unix:

```sh
acheflow-server --native-links --config /path/to/acheflow-server.json
```

A launcher writes the exact server forwarding URL to `acheflow-server
--forward-stdin` and closes stdin. The invocation hands off to an existing
native-link server in the same runtime directory. If none is running, it elects a
primary and starts the server with its usual configuration flags; `--config` and
listener overrides can be supplied for this cold start. The receiver starts after
server listeners and the control host are initialized. There is no active
workspace on a fresh server until the client configures one, so an initial event
then reports NotReady rather than configuring a workspace or enabling a guest.

Ordinary server startup without either flag keeps its existing standalone
behavior and does not claim the native-link endpoint. Only one opted-in server
can own `server.sock` in a runtime directory. Use distinct private
`ACHEFLOW_RUNTIME_DIR` values for independent local server installations; this
directory selects the recipient, not a URL parameter. Client and server endpoints
can coexist in one runtime directory.

The server accepts declared `event` actions only. It resolves the currently active
workspace, enabled extension and live instance, then submits the existing
`forward_extension` control command with the observed revision and a fresh intent
ID. Workspace, revision, manifest and instance checks still apply. The receipt's
expiry/cancellation is checked after waiting for the command lock and again after
waiting for the guest inbox lock. Server shutdown cancels an outstanding native
wait; an unconfirmed outcome is not retried automatically. Missing/disabled/not-ready
recipients and rejected events produce fixed diagnostics without the original URL.

This ingress adds no remote endpoint and does not execute shell commands or
provider authorization. The same-user private socket is its local access boundary;
provider credentials and OAuth callbacks remain client-side. Signed server URL
handlers need signing/installation validation and production packaging remains
DL-11. The server routing tests use
real independent WASM guests; full cross-process activation is still pending.

## Browser-to-application fallback

The current platform schemes are `acheflow-client://fwd/<id>/<action>/...` and
`acheflow-server://fwd/<id>/<action>/...`. They are transport aliases: the native
launcher converts only its own scheme to the matching canonical HTTPS origin,
then uses the same parser and session checks. IPC and guest APIs continue to
require canonical HTTPS addresses. No application claims the system-wide `https`
scheme. Linux desktop entries use a small `acheflow-link` adapter with fixed
sibling binaries and stdin handoff; the URI appears in the adapter's OS-supplied
arguments, but never in its child's arguments or diagnostic output.

The generated HTTPS fallback page opens an application only after the user clicks
Open. It removes path/query/fragment from the current history entry and retains
the URL in memory, without analytics, storage, third-party assets or network I/O
from its JavaScript. The installation action goes to `acheflow.com` without the
callback. Refreshing after history cleanup loses the pending payload; users must
return to the original link. The initial HTTPS request can still be visible to
the hosting infrastructure, so access/CDN/debug logging must omit callback URLs.

Custom schemes are not a verified app/domain association: another application can
register the same scheme. Prefer signed HTTPS universal links for provider flows
that support them; keep correlation/state and provider-specific protections.
The shared `acheflow://` dispatcher and a combined executable are explicitly future
DL-13 work; no URI layout or execution flags for that design are finalized here.

Implementation progress: [task list](../tasks/todo-desktop-links.md).
