# Native link installation

Acheflow ships one application, one `acheflow` executable and one `acheflow://` scheme.
Client URIs use `acheflow://client/fwd/<extension>/<action>/...`; server URIs use
`acheflow://server/fwd/<extension>/<action>/...`. Canonical HTTPS equivalents are
`https://app.acheflow.com/fwd/...` and `https://app.acheflow.com/server/fwd/...`.
OAuth callbacks remain client-only. See [forwarding.md](forwarding.md) for the
runtime security/lifecycle contract and [desktop-installation.md](desktop-installation.md)
for complete packages.

## macOS development bundle

```sh
cargo build -p acheflow --bin acheflow --locked
python3 native/macos/build-dev-app.py \
  --output target/Acheflow.app --bundle-id com.acheflow \
  --binary target/debug/acheflow
```

Choose a new output path. The app has one executable,
`Contents/MacOS/acheflow`, and registers the shared scheme and browsing user
activities on the main bundle. There is no nested launcher application.

At startup the executable parses explicit arguments before creating any UI. A
normal macOS app launch first installs its AppKit event receiver and waits for the
launch intent. A URL launch selects its recipient before creating a window; a
server link starts the server runtime without constructing the studio. Ordinary
launch creates the studio after that selection. Later URL events use bounded
background IPC to reach the selected recipient. A missing recipient can start
another process of the same executable; URLs travel through stdin without a shell
or payload logging. One executable does not require client and server to share a
process lifetime.

`ACHEFLOW_CONFIG` may select server configuration; otherwise an OS-activated
server uses `~/Library/Application Support/Acheflow/Server`. `ACHEFLOW_RUNTIME_DIR`
isolates local client/server receivers. Ordinary standalone servers can opt in
with `acheflow --server --native-links`. URL parameters cannot choose executables
or profiles.

The development builder copies only the named Rust binary. The distribution
packager adds extensions, web assets and runtime libraries. Development identities
default to `com.acheflow.development`; generated association metadata must match
the actual installed main bundle identity.

Open the installed app for LaunchServices discovery and verify scheme registration
on the target macOS. Build scripts do not change the user's default handlers.
Temporary development bundles, ad-hoc signatures and local IPC tests do not
establish installed signed HTTPS association.

## Apple HTTPS association

The app host and main bundle require matching two-way association as described in
[Supporting associated domains](https://developer.apple.com/documentation/xcode/supporting-associated-domains).
The application's AppKit delegate receives browsing activities as described in
[Supporting universal links](https://developer.apple.com/documentation/xcode/supporting-universal-links-in-your-app).

```sh
python3 native/links/build.py --output target/acheflow-links-signed \
  --apple-team-id YOURTEAMID --handler-id com.acheflow
```

Replace the example with the real 10-character Team ID. If the Application
Identifier Prefix differs, supply `--apple-app-prefix`. Generated
`macos/links.entitlements` identifies the main app and `applinks:app.acheflow.com`.
One AASA entry claims only `/fwd/*` and `/server/fwd/*`; the browser root and
ordinary browser navigation remain unclaimed.

Enable Associated Domains for the main App ID, obtain its matching profile, and
embed it in the app's `Contents/embedded.provisionprofile`. Sign runtime libraries
first, then the app with its generated entitlements. Verify signatures, identifiers
and granted entitlements before notarization and installation. Generated metadata
does not establish granted Apple capabilities. Signing/profile access and
installed universal-link verification remain external rollout requirements.
Local packaging uses ad-hoc signatures only.

## Shared-domain HTTPS fallback

```sh
python3 native/links/build.py --output target/acheflow-links
```

The output contains only `app.acheflow.com/` plus a Caddyfile. Client requests
`/fwd/*` serve `forward.html`; server requests `/server/fwd/*` serve
`server-forward.html`. Both load isolated `/_acheflow-links/` assets. The generator
never replaces the browser client's `index.html`. Merge these files and routes
with the browser deployment. Without a real Team ID no AASA/entitlement is emitted.

Caddy uses `ACHEFLOW_LINK_ROOT` as the parent of the host document root. It
preserves the original URL, serves association metadata separately as JSON, and
sets no-store, no-referrer, CSP, frame and MIME restrictions for link resources.
Caddy itself remains a deployment dependency; its configuration is not validated
by local artifact generation alone.

For Cloudflare Pages Direct Upload, upload the host root including `_headers`,
`_redirects` and isolated assets. The two 200 rewrites target extensionless
`/forward` and `/server-forward`, avoiding Pages' `.html` redirects. Deploy both
routes and the shared assets together with the matching installed scheme handler.
Generated local files do not establish live fallback or AASA support.

Serve AASA over valid public HTTPS without redirects or authentication and check
Apple CDN propagation. Do not enable analytics or callback-bearing access/debug
logs. The initial HTTP request reaches hosting infrastructure; client history
cleanup cannot erase it. Fallback activation requires an explicit click and
retains the URL only in memory. The Get Acheflow link carries no callback data.

## Linux registration artifacts

Install the unified `acheflow` executable in a trusted desktop PATH directory and
`native/linux/com.acheflow.links.desktop` in `~/.local/share/applications`:

```sh
xdg-mime default com.acheflow.links.desktop x-scheme-handler/acheflow
```

The single `%u` argument follows the
[Desktop Entry specification](https://xdg.pages.freedesktop.org/xdg-specs/desktop-entry/latest/exec-variables.html).
It invokes `acheflow --forward` without a shell or general HTTPS handler. OS-supplied
URI arguments are sensitive in process/debug tools. Native parser, manifest,
workspace, instance and callback checks still apply. Linux desktop registration
requires validation on Linux. Windows activation is not implemented and no Windows
handler is advertised.

## Migration and rollout

Replace the separate client/server apps and registrations with the unified app
and its main-bundle registration. Retire `acheflow-client` / `acheflow-server` handlers and the old
client/server forwarding domains; the Rust parser rejects them. Restore any
previous handler preference deliberately when uninstalling, without modifying
unrelated applications. Preserve workspace files and credentials outside the app.
Provider callback registrations on `https://app.acheflow.com/fwd/...` stay unchanged.
Replace hosted fallback JavaScript together with installed handlers: old deployed
pages emit the old scheme until updated. Domain/signing/default-handler changes
are separate external rollout actions. Track their product-level completion in
the [roadmap](roadmap.md), without publishing live registration/test identifiers.
