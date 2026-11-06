# Desktop lifecycle and extension forwarding

## Agreed scope

Make the native Acheflow client a complete utility that can run and monitor its
local server. Give installed clients and servers explicit forwarding addresses,
and keep extension connection setup self-contained in each installation.

| Address | Role |
| --- | --- |
| `acheflow.com` | Product website |
| `app.acheflow.com` | Browser client and target deep-link domain (2026-11-05 revision) |
| `app.acheflow.com/fwd/<extension_id>/<action>/...` | Target extension link contract |
| `server.acheflow.com/fwd/<extension_id>/<action>/...` | Existing server transport, pending migration |

The server forwarding domain identifies a link recipient, not the user's actual
server API address. Forwarding is a transport, not permission to execute an action.
OAuth/provider credentials remain on clients under the v2 extension contract.

### Confirmed authorization requirement — revised 2026-11-06

Users register an application with their selected provider and enter its Client ID
and any required local application secret in the extension card before Connect.
No application credentials are embedded or inserted automatically. This lets users
choose the application owner and quota. Connect stays disabled until required
inputs are present; sign-in/consent then obtains the user's own tokens. The extension
manifest owns fields, registration instructions and required permissions. A visible
Disconnect replaces Connect after successful authorization.

The host supplies a trusted, installation-derived link base:
`https://app.acheflow.com/fwd/<extension_id>`. The extension declares only its
action/path suffix, e.g. `oauth/callback`; it does not choose the domain or another
extension's ID. The full Twitch example is
`https://app.acheflow.com/fwd/twitch/oauth/callback`. Preserve the existing
`/fwd/<extension_id>/<action>/...` shape; do not introduce a literal
`/extension/oauth/callback` route. Provider consoles still register the full URL.

Deep-link delivery preserves the bounded path/query/fragment and routes only to
the installed, enabled extension with a matching pending authorization. Extension
identity in the path and single-use `state`/workspace/instance checks are all
required; receiving a link is not authorization. OAuth callbacks always target
the client. DL-13 defines explicit recipients for other links and migrates the
old domains/schemes to the shared domain alongside the unified executable.

Provider protocols remain under extensions. Google Desktop uses its supported
code/S256 PKCE flow with a free loopback port, behind the same Connect action;
Google browser keeps Identity Services. Use the target HTTPS callback where the
provider/client type supports it. Browser-client callbacks must remain separate
from native activation. Provider consent and required browser/OS confirmations
cannot be silently skipped.

Acceptance: users connect with their own registered applications on native
and browser clients; changing the server port or occupying the former 21941 port
does not prevent native sign-in. Test cancellation, duplicate/late/wrong-extension
callbacks, application changes, local-only token handling and browser/native separation with
real WASM guests plus live provider consent. Installed app/domain association and
hosted fallback must be verified before claiming automatic return. The current
implementation checkpoint is recorded in the task list. The former client domain
is removed, with no compatibility alias because no release has shipped. Hosting,
signing and DL-13's executable/server dispatch remain
required work.

## Ordered implementation

### DL-01 — Unambiguous extension installations

Reject every installation involved in a duplicate manifest ID within a recipient's
catalog, including a third occurrence. Report installation locations; never combine
one installation's WASM with another's browser authorization module. Preserve
unrelated installations. Apply the rule to native, bundled and server catalogs.

Acceptance: two/three duplicate fixtures are all unavailable; unrelated providers
remain available; errors identify conflicting sources. Verify using real bundled
provider WASM and temporary installation directories.

### DL-02 — Shared forwarding address contract

Parse only the intended HTTPS hosts and `/fwd/<extension_id>/<action>/...` routes.
Keep the extension ID stable and validate path encoding, size and recipient. Keep
provider parameters opaque to the host and redact callback payloads from logs.

Acceptance: client/server recipients stay separate, ambiguous or malformed paths
are rejected, and the query/tail survive delivery. Verify parser boundary tests.

### DL-03 — Extension forwarding delivery

Declare supported forwarding actions in the versioned manifest/domain contract.
Deliver generic bounded link events only to installed, enabled live recipients.
OAuth callbacks must match a pending local transaction and its workspace/instance;
expired, cancelled, duplicate and unsolicited callbacks must not authorize accounts.

Acceptance: two independent real WASM guests receive only their own events; stopped
instances and switched workspaces reject late results. Rebuild guests before host
tests and verify existing authorization/cancellation regressions.

### DL-04 — Native client link activation

Handle cold-start and already-running URL activation, with a bounded local handoff
to the existing client. Provide clear errors for missing or conflicting recipients.
Keep browser UI navigation separate from native process activation.

Acceptance: an external link reaches the correct running/cold client without
creating an unintended second UI. Verify local process handoff and native launch.

### DL-05 — Native server link activation

Provide a server launcher/receiver that forwards links to its running local
instance. Keep provider authorization on clients; server guests remain pure logic.
Do not expose a generic remote command-execution endpoint.

Acceptance: a link reaches the intended server guest, cannot target a client guest,
and cannot bypass server authorization or instance lifecycle boundaries.

### DL-06 — Domain and installation integration

Package OS URL handlers and associated-domain metadata for the client/server.
Provide HTTPS fallback pages with explicit Open application and installation
actions. Keep the product website and browser client addresses distinct.

Acceptance: matching signed installations handle supported HTTPS links and missing
handlers show the fallback. Live DNS/hosting, signing and provider redirect
registration require the corresponding deployment credentials; local artifacts
and instructions must be prepared before that external rollout.

### DL-07 — Extension-owned application setup

Each provider supplies developer-console links, application-type instructions,
permissions, the exact active callback with a copy action, public Client ID fields
and only the local secret fields required by that provider. The shared UI renders
generic declarations. Keep loopback, device authorization and supported deep-link
strategies provider-owned; do not force every provider through a hosted callback.

Acceptance: two independent providers render their own setup and failures without
provider-specific UI/server code. Verify native/browser rendering and real WASM.

### DL-08 — YouTube streaming chat

Replace periodic chat list requests with the official `liveChatMessages.streamList`
transport. Preserve resume tokens, reconnect backoff, cancellation, deduplication
and provider-neutral bounded host I/O. Twitch continues using EventSub WebSocket.

Acceptance: streamed batches, interrupted/resumed streams and shutdown work with
bounded memory, no polling loop and no duplicate domain messages. Verify transport
and real guest fixtures; live accounts remain a separate credentialed check.

### DL-09 — Owned local server process

Resolve a compatible bundled server binary, launch without shell interpolation,
wait for API readiness, monitor exit and retain bounded diagnostic output. Track
ownership so existing independently started servers are not stopped implicitly.

Acceptance: successful readiness, missing binary, port conflict, startup timeout,
crash, stop and restart are exercised against real local subprocesses.

### DL-10 — Desktop Settings / Server

Expose start/stop/restart, actual process/API state, endpoint, ports, version,
recent logs and errors. Connect to a local server after it is ready. Persist opt-in
start-with-client and keep-running-on-client-exit preferences. Browser clients show
connection availability and never offer local process launch.

Acceptance: native controls follow process state, externally owned processes are
identified, preference restoration works, remote targets remain configurable and
browser UI remains usable. Verify focused UI behavior and final screenshots.

### DL-11 — Desktop distribution

Include compatible client/server binaries, handlers, extension artifacts and
launch defaults. Document installation and standalone/remote-server operation.

Acceptance: an installed desktop package can start its bundled server and connect
without developer tools. Verify package layout and a native installation smoke test.

### DL-12 — Provider application setup and live Connect

Expose each extension's declared application fields before Connect, require complete
inputs, and remove embedded/default application requisites. Document registration,
exact callback/origin, Testing allowlists and required scopes. Keep native application
secrets in the local credential store and browser Google sign-in secret-free. Preserve
provider-neutral host code and independent credentials per workspace/server/application.

Acceptance: verify two independent real WASM providers, button readiness, editable
application IDs, local secret drafts, successful consent/validation, restart and visible
Disconnect/cancellation. Record live Google failures separately from hosted callback
success. Maintainer registrations remain test fixtures, not user defaults. Any quota
request requires measured evidence; separate Google Client IDs in one project do not
create independent project quota. Never shard one use case to bypass provider limits.

### Future — DL-13: Unified executable and URI dispatcher

Use one `acheflow://` scheme with an explicit client/server recipient carried
inside the URI. Consolidate launch into one executable supporting server-only
operation or a desktop client that starts its server when needed. Define the
exact URI layout and flags in that task; do not infer them from the temporary
platform adapters. Preserve recipient, workspace, instance and callback checks.
Plan migration from the separate client/server scheme handlers and forwarding
domains to `app.acheflow.com`, retaining `/fwd/<extension_id>/<action>/...` and
provider-neutral routing. Explicitly define recipient selection and browser/native
separation; OAuth stays client-only. Product pages remain on `acheflow.com`.
This is future implementation work, not support in the current binary package.

## Delivery discipline

Track progress in [todo-desktop-links.md](todo-desktop-links.md). Keep existing
authorization/layout work intact. Check off a task only after its acceptance
criteria have been met. Future service approvals and DNS/signing operations remain
explicit external rollout dependencies, not claims of completed local behavior.
