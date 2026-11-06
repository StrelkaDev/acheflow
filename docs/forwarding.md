# Installed application forwarding

The forwarding address contract is implemented by
`acheflow_extension_api::forwarding::ForwardLink::parse_for`. Manifest-declared
delivery is available for live client sessions and server guests. Unix desktop
clients also provide a local activation receiver. Generated fallback and association
artifacts require coordinated deployment and signed installation validation. Parsing
an address alone does not authorize an extension action.

## Addresses

| Address prefix | Recipient |
| --- | --- |
| `https://app.acheflow.com/fwd/` | Installed desktop client |
| `https://app.acheflow.com/server/fwd/` | Local server |
| `acheflow://client/fwd/` | Installed desktop client |
| `acheflow://server/fwd/` | Local server |

Each route continues with `<extension_id>/<action>/...`. Client callback URLs
remain unchanged, e.g. `https://app.acheflow.com/fwd/example/oauth/callback`.
The server's explicit `/server` prefix carries routing outside provider payloads.
Query/fragment fields never select the recipient. Custom URIs normalize into the
matching HTTPS form before private IPC or domain APIs receive them.

`acheflow.com` remains the product website. Ordinary `app.acheflow.com` browser
routes are separate from installed-app activation. The former client/server
forwarding domains and `acheflow-client://` / `acheflow-server://` schemes are
rejected. No release requires compatibility aliases. The forwarding host is not
the user's server API endpoint. Canonical authorities are required, without ports,
user information or alternate host spellings. Wrong-recipient delivery fails closed.

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
{
  "forwarding": [{"id":"oauth","mode":"callback"}]
}
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
validation for the configured installation; application credentials have no
embedded defaults.

## Unified native activation

The production executable is `acheflow`:

```sh
acheflow                                      # desktop; local server on demand
acheflow --client --show                       # explicit desktop
acheflow --server --native-links --config /path/to/server.json
acheflow --server --help                       # existing server flags
acheflow --forward-stdin                       # route one exact address from stdin
```

`--forward <address>` and a single address argument also work. OS adapters use
stdin and close it after writing, so callbacks do not enter child process
arguments. Input is bounded to 16 KiB and trailing whitespace is rejected.
Explicit `--client` rejects server links; `--server --forward-stdin` rejects client
links. `--help`/`--version` initialize neither a GUI nor a native receiver. The
separate `acheflow-ui` and `acheflow-server` Cargo entrypoints remain development
wrappers around the shared libraries; distribution includes only `acheflow`.

On Unix, `acheflow-launcher` elects one client and one opted-in server per private
runtime directory. Warm requests hand off and exit; cold requests initialize only
the requested recipient. The UI restores/focuses its window and selects Extensions.
Missing/conflicting/disabled/not-ready installations produce fixed errors without
showing the URL. A fresh server has no configured workspace and rejects forwarding
rather than enabling an extension. The receiver uses the existing control command
boundary, checking workspace/revision, live instance, cancellation and expiry.

The default runtime directory is `<system temporary directory>/acheflow-<uid>`;
`ACHEFLOW_RUNTIME_DIR` selects a private isolated profile. URLs cannot select that
profile. Each receiver uses its own lock and mode-0600 socket in an owned mode-0700
directory. No TCP activation endpoint or callback payload file is created. Stale
owned sockets are recovered under the lock; symlinks and regular files are rejected.
Handoffs have bounded frames, queues and receipt deadlines. Success means that a
live session accepted delivery, not provider authorization. Unknown outcomes are
not automatically retried. Windows native activation remains unsupported.

Standalone `--server` preserves ordinary multi-server operation; add
`--native-links` to participate in local dispatch. The desktop's owned child uses
`--server --native-links --desktop-managed`, sharing the client's runtime directory.
Its lifetime/diagnostic pipe remains separate from link stdin. An independent or
detached server remains externally owned. Native callback correlation and pending
browser operations retain all cancellation/replay protections.

## Installation and fallback

The macOS app's single executable receives both URI recipients and supported
HTTPS user activities through its own AppKit delegate. Cold activation selects
the recipient before UI construction; later events use bounded background IPC.
If the recipient is absent, the same executable can launch in that mode with the
URL supplied through stdin, without a shell, payload logs or disk storage.
Linux has one `acheflow` scheme desktop entry invoking `acheflow --forward %u`.
See [installation integration](native-links-installation.md) and
[desktop distribution](desktop-installation.md) for build, signing and registration.

The shared-domain fallback uses `/fwd/*` for client links and `/server/fwd/*` for
server links. It never replaces browser `index.html` or activates ordinary browser
navigation. Opening requires a user click; history path/query/fragment are removed
before leaving the page. Payload stays only in memory; refresh loses it. There are
no analytics, third-party assets or JavaScript network requests. The installation
link goes to `acheflow.com` without the callback. Hosting access/debug logs must
omit callback URLs: browser history cleanup cannot erase the initial HTTP request.

The site generator emits one optional AASA identity covering only those two paths
and one associated-domain entitlement for `app.acheflow.com`. Custom schemes alone
are not verified domain association; all authorization checks remain necessary.
Local builds do not deploy fallback pages, register handlers or establish signed
universal links. Replace older fallback assets together with the shared-scheme
handlers before rollout; old server-domain links are no longer emitted.

See [installation](native-links-installation.md) and [remaining work](roadmap.md).
