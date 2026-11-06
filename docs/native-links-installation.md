# Native forwarding installation and hosting

These are DL-06 artifacts, not a signed release. The app return page is deployed
as recorded below. Server/client activation and delivery follow
[forwarding.md](forwarding.md). The generator itself does not change DNS, publish
pages, install handlers or sign software automatically.

## macOS bundles

For a complete client/server distribution with extension artifacts, browser UI and
relocated runtime libraries, use [desktop-installation.md](desktop-installation.md).
The commands below remain minimal development adapters for isolated link tests.

Build the matching Rust binaries, then create new unsigned output bundles:

```sh
cargo build -p acheflow-ui -p acheflow-server -p acheflow-launcher --bins
python3 native/macos/build-dev-app.py \
  --output /private/tmp/Acheflow.app --recipient client \
  --bundle-id com.acheflow.client --ui-bin target/debug/acheflow-ui
python3 native/macos/build-dev-app.py \
  --output /private/tmp/AcheflowServer.app --recipient server \
  --bundle-id com.acheflow.server.links --server-bin target/debug/acheflow-server
```

The client GUI retains its own app identity. Its URL adapter is
`Acheflow.app/Contents/Helpers/AcheflowLink.app` with identity
`com.acheflow.client.links`. The server app is itself the AppKit URL adapter,
identity `com.acheflow.server.links`, with the Rust server at
`Contents/MacOS/acheflow-server`. Each declares only its own custom scheme and
`NSUserActivityTypeBrowsingWeb`; neither registers as a general HTTPS handler.
The adapters pass URLs over stdin and select fixed bundled executables without a
shell. Finder launches use `~/Library/Application Support/Acheflow/Client` or
`…/Server` as their working directory; the standalone Rust client's existing
workspace storage location is unchanged. `ACHEFLOW_CONFIG` may specify an absolute
server configuration path. Separate profiles may set `ACHEFLOW_RUNTIME_DIR`.

Without `--bundle-id`, builds use development identities. If those identities are
used for domain testing, pass their exact handler IDs to the site generator too.
The development bundle copies the named binary only. The DL-11 packager adds the
matching owned server, extensions, web assets and runtime libraries. Developer ID
signing and notarization remain external rollout requirements.

After preparing the final signed installation, install it in Applications and
launch its URL adapter once so LaunchServices discovers it. For the client this
means opening the nested `AcheflowLink.app`; for the server, opening its main app.
Then verify the exact scheme-to-adapter registration on the target macOS version.
This repository does not run registration or change the user's default handlers.
Test from an actual Applications installation. On macOS 27.0.1, the DL-12 helper
could be present in LaunchServices' claims while remaining unavailable for URL
selection under `/private/tmp`; the same ad-hoc signed bundle under the user's
Applications directory resolved and received the Twitch OAuth callback.

## Apple HTTPS association

Each host needs a matching AASA file and its adapter needs the associated-domains
entitlement. Apple describes this two-way binding in
[Supporting associated domains](https://developer.apple.com/documentation/xcode/supporting-associated-domains).
Universal links arrive as a browsing `NSUserActivity`; the Swift adapter handles
that callback as documented in
[Supporting universal links](https://developer.apple.com/documentation/xcode/supporting-universal-links-in-your-app).

Generate with the real Apple Team ID and registered handler bundle IDs:

```sh
python3 native/links/build.py --output /private/tmp/acheflow-links-signed \
  --apple-team-id YOURTEAMID \
  --client-handler-id com.acheflow.client.links \
  --server-handler-id com.acheflow.server.links
```

`YOURTEAMID` is an example argument; substitute your actual 10-character ID.
If the registered Application Identifier Prefix differs from the Team ID, also
supply `--apple-app-prefix`. Match it to the provisioning profile's application
identifier. Generated entitlements in `macos/` declare only that recipient's
`applinks:<host>` domain and identity; AASA claims only `/fwd/*`. It does not claim
the website, browser client, other subdomains or unrelated paths.

Enable Associated Domains for each adapter's App ID, obtain matching provisioning
profiles and embed each profile in its adapter's `Contents/embedded.provisionprofile`.
Use an appropriate Apple signing identity to sign inner Rust executables first,
then the adapter with its generated entitlements, then the enclosing client app.
Verify signatures, identifiers and granted entitlements before notarization and
installation. Avoid signing only the outer app or treating generated entitlements
as proof that Apple granted the capability. Actual identity/profile access and
installed universal-link validation are external rollout requirements.

## HTTPS hosting

For fallback pages without an Apple identity, omit the signing arguments:

```sh
python3 native/links/build.py --output /private/tmp/acheflow-links
```

The output contains `app.acheflow.com/` and `server.acheflow.com/` document roots
and a Caddyfile. The former client domain is not generated. The app host uses
`forward.html` and `/_acheflow-links/` assets; it never replaces browser `index.html`
or claims `/` as a native route. Merge the generated app routes/files with the
actual browser-client deployment when sharing this host. Without a real Team ID,
the builder emits no AASA file or
Apple entitlement. Do not publish test identity metadata from fixtures.

Copy the host document roots to a deployment directory, set `ACHEFLOW_LINK_ROOT`
to that directory (for example `/srv/acheflow-links`), and use the generated
Caddyfile with the corresponding DNS/TLS configuration. The Caddyfile serves
`/fwd/*` as the fallback page while keeping the original browser address for JS
validation; association metadata is a separate JSON resource. Other missing paths
return 404. It sets CSP, no-referrer, no-store, frame and MIME restrictions, and
adds no access-log directive. Caddy itself is a deployment dependency; its config
has not been executed in this workspace.

For Cloudflare Pages Direct Upload, upload only the contents of the app document
root, including `_headers`, `_redirects` and `/_acheflow-links/`. The generated
200 rewrite keeps `/fwd/*` in the browser address. Its destination is `/forward`,
because Pages canonicalizes `.html` destinations with a redirect. Add the custom
domain in Pages and, with external DNS, point its CNAME at the project's Pages
hostname. This does not require moving the parent zone.

On 2026-11-05, `acheflow-app.pages.dev` and the REG.RU CNAME for
`app.acheflow.com` were deployed. Pages reports the custom domain Active. Both
Twitch and VK callback paths and all three assets return public HTTPS 200; the
Twitch page matches the generated bytes, keeps the original URL, and serves the
required no-store/no-referrer/CSP headers. No AASA was published: signed identity
and installed universal-link delivery remain unverified.

Serve `/.well-known/apple-app-site-association` over valid public HTTPS as JSON,
without authentication or redirects. Validate each host independently, including
Apple CDN propagation. Do not enable analytics, request/Referer/body logging or
CDN traces containing callback URLs. The first HTTP request necessarily reaches
the hosting infrastructure when the OS/browser chooses the website. The page
scrubs the browser history entry only after loading; it cannot erase upstream logs.
The Get Acheflow link points to the product website and carries no callback data.

## Linux desktop adapters

Build `acheflow-link` with the command above. Install it alongside the actual
`acheflow-ui` and `acheflow-server` binaries in a trusted directory on the desktop
session's PATH. The adapter resolves its child relative to its own executable,
not by a caller-supplied path. Install the two `.desktop` files from `native/linux`
in `~/.local/share/applications`, then explicitly register:

```sh
xdg-mime default com.acheflow.client.links.desktop x-scheme-handler/acheflow-client
xdg-mime default com.acheflow.server.links.desktop x-scheme-handler/acheflow-server
```

The desktop `Exec` entry uses a single `%u` URL argument, following the
[Desktop Entry specification](https://xdg.pages.freedesktop.org/xdg-specs/desktop-entry/latest/exec-variables.html).
It uses no shell or general HTTPS handler. Treat OS-supplied URI arguments as
sensitive in system process/debug tooling. Native parsing, manifest declarations,
callback correlation and workspace/instance guards still apply. Reverting a
registration requires restoring the user's previous scheme-handler preference;
do not remove an unrelated application's preference. Linux desktop integration
requires verification on a Linux desktop and does not claim verified HTTPS-domain
association.

## Current verification boundaries

Local checks cover Rust normalization/rejection, fallback payload preservation,
explicit-click behavior, history cleanup, generator identity separation, native
Clippy/builds, Swift type-check and unsigned macOS bundle layout. The browser was
checked at desktop and 320px widths. Its valid-state fixture supplied a dummy
canonical HTTPS link and replaced OS navigation with a test callback; it did not
launch an installed handler. The actual localhost page rejects its noncanonical
origin as expected, with no console warnings/errors.

Signed macOS installation, live AASA delivery, Linux desktop registration
and complete cold/warm cross-process activation remain unverified. Windows has no
native activation transport yet, so no working Windows registration is generated.
A Windows packaged URI handler must be implemented before publishing matching
web-to-app claims. DL-06 stays open for those acceptance checks.
DL-12 additionally verified a warm macOS custom-scheme callback from Safari to the
installed ad-hoc bundle, real Twitch token validation, and authorization restoration
after restart. It does not establish signed HTTPS association or other platforms.

Future DL-13 introduces one `acheflow://` scheme and one executable with an
explicit recipient and server-only/client-with-server modes. That design is
recorded separately; current schemes are temporary transport aliases.
