# Development

Use the repository's pinned Rust toolchain, host lockfile and separate guest
lockfile. [Getting started](getting-started.md) covers prerequisites and launch
modes; [architecture](architecture.md) maps the packages.

## Build and run

```sh
cargo fetch --locked
cargo fetch --manifest-path extensions/Cargo.toml --locked
python3 extensions/build.py
cargo build -p acheflow --release --locked
cargo run -p acheflow --release --locked
cargo run -p acheflow --release --locked -- --server
```

The final two commands are alternative launch modes. The desktop manages its own
server when needed; do not start another server on the same ports unnecessarily.
For browser UI assets, install the pinned Trunk version and build:

```sh
cargo install trunk --version 0.22.1 --locked
trunk build
```

Trunk defaults to optimized WASM. `trunk serve` uses localhost:8081 and needs a
separate backend allowing that origin. Native development entrypoints remain
available, but installation docs and packages use the unified `acheflow` binary.

## Standard checks

On a fresh checkout and after guest/SDK changes, build guests before host builds
or tests. Keep source, manifests and generated local WASM artifacts matched.
These build outputs, including the domain test fixture, are ignored by Git and
must not be committed. CI must run the guest build before host checks; macOS
packaging runs it automatically unless explicitly reusing an existing build.
Do not substitute native guest code for real-WASM integration tests.

```sh
python3 extensions/build.py
cargo fmt --manifest-path extensions/Cargo.toml --all -- --check
cargo clippy --manifest-path extensions/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path extensions/Cargo.toml --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p acheflow-ui --lib --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
node --test native/links/page.test.mjs
python3 -m unittest discover -s native/links -p 'test_*.py'
python3 -m unittest discover -s native/macos -p 'test_*.py'
trunk build
```

Real-media/GPU tests are opt-in and need FFmpeg, a GPU adapter and loopback sockets:

```sh
cargo test -p acheflow-server --locked -- --ignored --nocapture
cargo test -p acheflow-composition --features headless --locked -- --ignored --nocapture
```

Some native credential tests use the OS vault and should run with isolated fixture
records. A sandbox blocking local listeners cannot validate process/IPC tests.
Run affected checks after changes; reuse successful evidence only for unchanged
code/dependencies/environment. Documentation-only edits need link, command, content
and privacy checks rather than an unrelated media-suite rerun.

## Packaging and UI checks

Use [desktop packaging](desktop-installation.md) for the macOS archive builder and
relocated-installation smoke test. Its ad-hoc signing is distinct from public
Developer ID/notarization and HTTPS app association.

Check actual native/browser rendering when changing UI behavior. Verify state and
numeric values with focused DOM/tests, and capture final visuals when layout is
relevant. Do not treat a compiled WASM file as proof of a working GPU or live provider.
[UI integration](ui.md) covers rendering/input and frame-pacing diagnostics.

## Repository context

Follow [AGENTS.md](../AGENTS.md) and the [extension contract](extension-contract.md).
Use Repowise for code relationships, risk and architectural context; verify changed
behavior in the owning source. Refresh uncommitted context when needed:

```sh
repowise status
repowise doctor
repowise update --working-tree --no-agents
```

An unindexed checkout can initialize structural context without editor/hook changes:

```sh
repowise init . --yes --no-prose --no-editor-setup --no-agents --no-claude-md --no-hook --no-save-key
```

Project MCP settings are in `.codex/config.toml` and `.mcp.json`. Start clients from
the checkout; machine-specific overrides belong in user configuration.
Do not commit, merge or rewrite commits without an explicit request.

## Documentation and verification data

The root README introduces the studio and launch paths. `docs/README.md` indexes
current guides; `docs/roadmap.md` is the single summary of unfinished product work.
Keep architecture decisions in `docs/decisions/`, clearly marking superseded choices.
Remove completed implementation plans and session logs when current guides replace
them; Git history retains their development context.

Use obvious placeholders in examples. Do not put real application/client/account/
channel/project IDs, user names, email addresses, callback payloads, secrets,
workspace/session IDs, process IDs or machine-specific paths from live checks into
documentation. Do not turn test-account configuration into defaults. Protocol
versions, schema field names, documented limits and public project/source links
are legitimate technical references. Keep private verification data outside tracked
docs and remove it from any artifact prepared for sharing. Scan current Markdown
and validate local links before finishing a documentation change.
