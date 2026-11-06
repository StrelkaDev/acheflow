# Developing an extension

Follow [the v2 contract](../docs/extension-contract.md) and
[the protocol/reference guide](../docs/extensions.md).

Client providers remain disabled by default. Their real WASM guests own account
panels and authorization; credentials stay local to the client. Public settings
are scoped to workspace + extension and never include secret values. See
[local authorization](../docs/extension-authorization.md).

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

For application setup, declare `application_setup` using the shared
`acheflow-extension-api::application_setup::ApplicationSetup` schema. Its
`settings` entries reference public manifest setting IDs, and native `secrets`
reference declared secret slots. Each platform supplies its own instructions and
optional active address source. The SDK emits generic setting/copy controls; the
host resolves them against the current installed/enabled extension. Another
provider needs only its manifest and guest, with no shared UI or server edit.

Use [provider setup](../docs/provider-setup.md) for user-owned application registration.
Do not include live application/account identifiers or callback data in examples.
