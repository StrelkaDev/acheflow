# Developing an extension

Follow [the v2 contract](../docs/extension-contract.md) and
[the protocol/reference guide](../docs/extensions.md).

The [YouTube](youtube/README.md) and [VK Video Live](vklive/README.md) guests use
manual OAuth token entry through manifest-declared secret fields. They remain
disabled by default. Credentials are scoped to workspace + extension and held in
server memory until restart; public settings and workspace exports omit them.

1. Copy the small `sample/` Rust crate into your own directory and give it a unique
   package/manifest ID. Add its crate to this Cargo workspace.
2. Declare only required settings, subscriptions, capabilities and external origins
   in `manifest.json`. Use a local `.wasm` module filename.
3. Implement `State::handle(serde_json::Value) -> Vec<Value>` and export it with
   `guest_sdk::export!(State)`. Keep service-specific parsing and OAuth here.
4. Emit channel descriptors to expose chat/poll destinations in the shared UI.
   Match commands using `type=command` and `name`, publish typed domain payloads,
   correlate results by request/poll ID, and respect lifecycle/queue limits.
5. Run `python3 extensions/build.py` (Python 3.8+, Rust wasm32 target), then host
   and guest checks. The script discovers manifests automatically. No server/UI
   change is needed to build or install another provider.

`fixture/` has no installation manifest and is only a host integration-test module.
Its binary is installed under the server's test fixtures by the same build script.
Check in version-matched WASM artifacts along with source/lockfiles so a host can
load installations without requiring an on-server compiler.
