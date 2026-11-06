# Desktop installation

The macOS distribution contains `Acheflow.app` (desktop client with its matching
local server) and `Acheflow Server.app` (optional standalone server). Install the
client to manage a local server from Settings / Server, or use it to connect to
an existing remote server. Install the standalone app only on a machine that
needs an independently running server. Both use the same package version.

## Install and run on macOS

1. Extract the ZIP and copy `Acheflow.app` to Applications. For standalone
   operation, also copy `Acheflow Server.app`. Keep each `.app` intact: moving
   individual executables separates them from their runtime libraries and assets.
2. Open Acheflow, then Settings / Server. Choose **Start server**. The client
   connects after the bundled server reports readiness. The initial ports are
   HTTP 21295 and RTMP 1935; change them here before starting if already in use.
3. Enable **Start local server with the desktop app** if desired. Both autostart
   and **Keep local server running when the app closes** default to disabled.
   Stop/Restart control only a server started by this client. An already running
   or detached server appears external and cannot be stopped by a different client.

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
rollout described in `docs/native-links-installation.md` in the source repository.
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

Open `Acheflow Server.app` to run an independent server with native-link reception.
It uses `~/Library/Application Support/Acheflow/Server/` for configuration and data,
serves the bundled browser UI on `http://localhost:21295`, and defaults standalone
HTTP/RTMP listeners to all interfaces. Quit this app to stop its standalone child.
Do not run it on the same ports as an owned server. For explicit listener/config
settings, launch the packaged binary from a terminal (no developer tools needed):

```sh
"/Applications/Acheflow Server.app/Contents/MacOS/acheflow-server" \
  --listen 127.0.0.1 --port 21295 --rtmp-listen 127.0.0.1 --rtmp-port 1935 \
  --config "$HOME/Library/Application Support/Acheflow/Server/acheflow-server.json"
```

Direct terminal launches use the caller's working directory. Packaged extensions
and browser assets are found in the app's `Contents/Resources/` directory;
`ACHEFLOW_EXTENSIONS_DIR` and
`ACHEFLOW_WEB_ROOT` override them. A local `extensions/` or `dist/` directory also
retains its existing development precedence. Create any desired data/config parent
directory before a direct terminal launch. Owned children always use loopback
listeners and their separate writable data directory.

## Link handlers

The client includes `Contents/Helpers/AcheflowLink.app`, which declares
`acheflow-client://`; the standalone app declares `acheflow-server://`. Open the
installed client helper once to let LaunchServices discover it, and open the
standalone server app if that recipient is needed. Verify the scheme registration
on the target macOS version. Packaging does not change default handlers or claim
general HTTPS links. Provider callbacks still require the installed/enabled
extension and a current authorization request. Associated-domain identities,
profiles, entitlements, live AASA hosting and signed activation remain separate
rollout work. The future unified executable/scheme remains DL-13.

## Build a distribution (maintainers)

From the source checkout on macOS with Rust/WASM target, Trunk and Xcode tools:

```sh
python3 native/macos/package.py --output target/acheflow-0.1.3-macos
```

This rebuilds the guests, release client/server and release browser UI, checks
exact binary versions, creates both apps, relocates non-system dynamic libraries,
copies available runtime license notices, computes the minimum OS requirement,
ad-hoc signs inside-out and verifies signatures. It creates a ZIP and SHA-256
sidecar beside the new output directory. It refuses an existing output/archive.
The script uses a file allowlist for extension artifacts and never copies the
checkout, source configuration or user home directory into an app.

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
