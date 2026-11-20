# Developing an extension

Follow [the v2 contract](../docs/extension-contract.md) and
[the protocol/reference guide](../docs/extensions.md).

Client providers remain disabled by default. Their real WASM guests own account
panels and authorization. Each executing host owns its credentials; the target
personal-action model keeps device accounts local. Public settings
are scoped to host + workspace + extension and never include secret values. See
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
Generated WASM artifacts are ignored; do not commit them. Build version-matched
artifacts during setup, CI and packaging, alongside source and lockfiles.

For application setup, declare `application_setup` using the shared
`acheflow-extension-api::application_setup::ApplicationSetup` schema. Its
`settings` entries reference public manifest setting IDs, and native `secrets`
reference declared secret slots. Each platform supplies its own instructions and
optional active address source. The SDK emits generic setting/copy controls; the
host resolves them against the current installed/enabled extension. Another
provider needs only its manifest and guest, with no shared UI or server edit.

Use [provider setup](../docs/provider-setup.md) for user-owned application registration.
Do not include live application/account identifiers or callback data in examples.

## Client and server parts

New packages declare `parts_version: 1` with `parts.client` and/or `parts.server`.
Use `kind: "provider"` for a client reactor or trusted server provider, and
`kind: "logic"` for pure server domain logic. Each part owns its settings,
authorization declarations, capabilities, supported actions and WASM entry point;
there is no sibling inheritance. See the governing
[part contract](../docs/extension-contract.md#package-parts-and-compatibility).

The guest SDK can export independent states from one artifact:

```rust
guest_sdk::export_part!(client_handle, Integration);
guest_sdk::export_part!(server_handle, Integration);
```

For typed wire inputs, use `export_part!(entry, State, Input, method)`.
Authentication helpers use the host-supplied `execution_host` to resolve the
embedded manifest. A server-only package needs no client entry or browser bundle.
Keep tokens out of package metadata; actual credentials/private storage are supplied
only by the executing host. Part support is separate from remote installation and
from the remaining client/server settings migration.
