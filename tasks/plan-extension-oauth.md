# Extension-owned authorization

Approved implementation direction: OAuth belongs to each installation under
`extensions/`. The client supplies rendering, local secret storage, bounded I/O,
browser/callback and cryptographic primitives. No provider-specific flow, scope
mapping, token response interpretation or renewal policy belongs in shared UI/server.

1. Replace the unfinished client-owned OAuth controller with extension-owned
   WASM authorization panels and actions. Keep reusable guest code under extensions/.
2. Add feature settings and minimal required scopes in each provider. Manual
   token input stays alongside Connect with and uses opaque client secret handles.
3. Connect the local panel reactor to native/browser clients, keeping credentials
   out of Workspace JSON, control actions, server snapshots and MCP events.
4. Verify refusal/expiry, changed permissions, cancellation, stale instance results,
   workspace/account isolation and two independent real WASM providers. Build guest
   artifacts before host tests, then check native/browser builds and rendered UI.

Confirmed configuration: native and browser UI; user-owned platform applications;
native Mac credentials in the OS keychain and browser credentials in memory only.
No commit or merge is authorized. Verification results are recorded separately.
