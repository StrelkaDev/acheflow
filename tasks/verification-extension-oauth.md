# Extension authorization verification — 2026-11-04

- Guest/WASM rebuild: `python3 extensions/build.py` passed; matching artifacts kept.
- Workspace regression suite: 176 passed, 0 failed, 11 ignored. The new ignored
  macOS integration test was also run explicitly: its real Keychain write/read/
  isolation/delete roundtrip passed and removed its own temporary record.
- Guest workspace: 4 tests passed. After splitting the account/guest handlers,
  affected real-WASM and UI regressions passed again: 12 tests, 0 failures.
- Native workspace/all-target, WASM UI and guest workspace Clippy passed with
  `-D warnings`. Formatting and `git diff --check` passed.
- State-guard mutation: changing `!=` to `==` made the existing-account regression
  fail. Correct source was restored and WASM artifacts rebuilt before final checks.
- Browser release build passed using the existing local wasm-bindgen/wasm-opt tools.
  Local visual checks covered Twitch, VK Video Live and YouTube account panels,
  manual token inputs, missing Client ID, changing scopes with enabled functions,
  public-setting restoration after reload and an empty error/warning console.
- Repowise working-tree index refreshed. Its review reports structural complexity
  warnings; the two initially large new handlers were split into focused functions.

Live account consent/renewal and platform application configuration were not tested:
the installation has no configured provider Client IDs/accounts for this check.
Account panels do not create/start an executor or change saved server grants.
See [authorization setup and limits](../docs/extension-authorization.md).
