# Desktop installation

The macOS distribution contains one `Acheflow.app`. Its `acheflow` executable
opens the desktop by default and supports headless `--server` operation. The
desktop starts the same executable as its managed local server when needed.

## Install and run on macOS

1. Extract the ZIP and copy `Acheflow.app` to Applications. Keep the app intact;
   its executable depends on the packaged runtime libraries and assets.
2. Open Acheflow. A workspace targeting its configured local endpoint starts a
   server if none is available. Settings / Server exposes state, Start/Stop/Restart
   and ports (initial HTTP 21295, RTMP 1935). A remote target stays remote.
3. **Start local server with the desktop app** also starts a local server for
   remote workspaces. **Keep local server running when the app closes** defaults
   to disabled. An existing independent or detached server remains external;
   Stop/Restart control only this client's child.

Use **Settings / Logs** for client connection/extension history and the selected
server's runtime log. Both update while the tab is open and support pause/search.
Local child-process output is also available there when its API cannot start.
See [local server diagnostics](local-server.md#settings--logs) for retention and
the server API boundary.

Rust, Python, Xcode and Homebrew are build tools, and are not needed to run the
installed apps or to start/connect to the local server. The package includes its
non-system OpenSSL libraries, guest WASM/manifests/browser authorization modules,
and browser UI. macOS system frameworks and a usable GPU are still required.
`package.json` records architecture and hashes; each app's `Info.plist` records
the minimum macOS version required by its executables and bundled libraries.
Build on the oldest supported macOS/SDK/runtime combination for wider compatibility.
These are native-architecture packages, not universal binaries.

FFmpeg is a separate **runtime** dependency for ingest, encoding and output. The
server can start and connect without it, but media operations require it. Install
an appropriate FFmpeg build from the [FFmpeg download page](https://ffmpeg.org/download.html).
Homebrew users may use `brew install ffmpeg`. The server discovers FFmpeg on PATH
or at `/opt/homebrew/bin/ffmpeg` and `/usr/local/bin/ffmpeg`. For another location,
set `ACHEFLOW_FFMPEG` to the absolute executable path in the server/client launch
environment. This distribution does not redistribute FFmpeg or its codec libraries.

The current archive is ad-hoc signed for local execution and has **no Developer ID
signature or notarization**. Public downloads require the signing/notarization
work described in [native link installation](https://github.com/StrelkaDev/acheflow/blob/main/docs/native-links-installation.md).
On a trusted local build, follow macOS's Open Anyway flow if prompted; never disable
Gatekeeper globally. A local smoke test is not proof of downloaded/quarantined or
signed universal-link behavior.

## Data, remote connections and standalone server

The client's workspaces and global lifecycle preferences live in
`~/.acheflow/workspaces.json`; its owned server data/configuration lives in
`~/.acheflow/server/`. Replacing the app does not replace these files. A client
started with an absolute `ACHEFLOW_WORKSPACE_FILE` uses that file and its sibling
`server/` directory instead. No user settings, credentials, tokens or provider
application secrets are copied into the distribution.

For a remote server, enter its HTTP(S) endpoint in Settings / Server and Connect.
Local startup preserves a remote endpoint until **Use local server** is chosen.
Configure the server's permitted browser origins and HTTPS when hosting a remote
browser client; desktop process control is available only for the client's own child.

For an independent server, run the same installed executable with `--server`.
Add `--native-links` to receive same-user server links; omit it for ordinary
multi-server operation. Explicit listener/configuration flags follow `--server`:

```sh
"/Applications/Acheflow.app/Contents/MacOS/acheflow" --server --native-links \
  --listen 127.0.0.1 --port 21295 --rtmp-listen 127.0.0.1 --rtmp-port 1935 \
  --config "$HOME/Library/Application Support/Acheflow/Server/acheflow-server.json"
```

Standalone defaults bind HTTP/RTMP to loopback (`127.0.0.1`). Remote access
requires an explicit bind-address change and an appropriate access boundary. Stop a terminal-owned server
with Ctrl-C. Do not use the same ports as another server.

Direct terminal launches use the caller's working directory. Packaged extensions
and browser assets are found in the app's `Contents/Resources/` directory;
`ACHEFLOW_EXTENSIONS_DIR` and
`ACHEFLOW_WEB_ROOT` override them. A local `extensions/` or `dist/` directory also
retains its existing development precedence. Create any desired data/config parent
directory before a direct terminal launch. Owned children always use loopback
listeners and their separate writable data directory.

## Link handlers

The main app declares the single
`acheflow://` scheme. URI authorities `client` and `server` choose their recipient;
HTTPS routes use `/fwd/*` and `/server/fwd/*` on `app.acheflow.com`. OAuth remains
client-only. The same executable receives macOS URL events and selects its mode
before creating the UI. `build-app.py` registers the finished app with
LaunchServices; the distribution packager does so only with `--register-links`.
Register the app at its final location again after moving it, or open the installed
app for LaunchServices discovery. Registration does not replace an existing user
choice of default handler. HTTPS universal links require the signed association
described below; scheme registration alone enables only `acheflow://` activation.

Old `acheflow-client://`, `acheflow-server://` and separate forwarding domains are
rejected. Replace the hosted fallback assets together with the installed handler
before rollout. Provider callback registrations remain unchanged. Associated-domain
identities, provisioning, signed activation and live fallback deployment remain
external rollout; local package verification does not establish them.

## Build a distribution (maintainers)

From the source checkout on macOS with Rust/WASM target, Trunk and Xcode tools:

```sh
python3 native/macos/build-app.py --output target/acheflow-app
open target/acheflow-app/Acheflow.app
```

This creates a complete ad-hoc-signed local app and registers its `acheflow://`
scheme at the resulting path. Use `--no-register-links` to leave LaunchServices
untouched. To create and register the app directly in Applications, use:

```sh
python3 native/macos/build-app.py --output /Applications/Acheflow.app
open /Applications/Acheflow.app
```

A path ending in `.app` is the final bundle path; other outputs are directories
containing `Acheflow.app`. Both builders accept this form. An existing app is not
overwritten: close it and move it aside before rebuilding. Applications must be
writable by the user running the build.

Both builders find Cargo and Trunk in `$CARGO_HOME/bin` (by default
`~/.cargo/bin`) as well as PATH. `--profile debug` selects debug builds for both
the native executable and the browser assets.

For a ZIP suitable for copying to another machine:

```sh
python3 native/macos/package.py --output target/acheflow-macos
open target/acheflow-macos/Acheflow.app
```

This rebuilds the guests, unified release executable and release browser UI, checks
exact versions, creates one app, relocates non-system dynamic libraries,
copies available runtime license notices, computes the minimum OS requirement,
ad-hoc signs inside-out and verifies signatures. It creates a ZIP and SHA-256
sidecar beside the new output directory. It refuses an existing output/archive.
Use `--register-links` to register this local app too. Alternatively,
`package.py --app-only` creates only the app without ZIP/inventory files.
The script uses a file allowlist for extension artifacts and never copies the
checkout, source configuration or user home directory into an app.

The app uses `achedev-logo` in Finder and the Dock, including direct executable
launches. Packaging copies the checked-in `achedev-logo.icns`. After changing the
shared SVG, regenerate it before building the executable and package:

```sh
swift native/macos/build-icon.swift
```

For an already validated local debug build:

```sh
python3 native/macos/package.py --profile debug --skip-build \
  --output target/acheflow-local-test
python3 -m unittest discover -s native/macos -p 'test_*.py'
python3 native/macos/smoke_package.py target/acheflow-local-test.zip
```

`--skip-build` requires matching current binaries, guest artifacts and `dist/`.
Use the default rebuild when producing a new distribution. Verify a copied or
extracted package outside the checkout, with an unrelated working directory:
open the client, start/stop its server, confirm bundled guests and browser assets,
then test standalone operation and both link recipients. Use isolated temporary
workspace/runtime directories for smoke tests. Linux desktop adapter templates
remain in `native/linux`; Linux packaging/desktop validation and Windows native
activation are not provided by this macOS packager.
