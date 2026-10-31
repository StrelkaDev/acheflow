# WASM extensions: agreed scope and draft boundary

Extensions are intended to provide both server logic and GUI panels.
The application's browser WASM build and guest WASM extensions are separate
artifacts. Building the GUI to WASM does not implement plugin execution.

Current code: `acheflow-extension-api` provides draft serializable manifest and
panel types. `acheflow-ui` renders these panel types with the same Dioxus/Blitz renderer
on native and web. No modules are loaded and no extension runtime is implemented.
API_VERSION is a draft schema version, not a stable executable plugin ABI.

Intended boundary:

- Server extensions run in a sandboxed WASM runtime on the server.
- GUI extensions describe panels and exchange events with the host. The host
  renders widgets, avoiding a second, platform-specific UI implementation.
- Client-side extension logic must work in both native and browser hosts.
- Do not pass UI-engine/Rust pointers or structs across the module boundary.
- Use an explicitly versioned message ABI; validate module manifests and messages.
- Runtime selection must verify browser compatibility, fuel/deadline handling,
  memory limits, cancellation and explicit host capabilities before adoption.
- Modules receive no filesystem, network access, stream keys or media buffers
  by default. High-throughput media processing needs a separately measured ABI.

Next milestone: choose a portable plugin ABI/runtime, implement a sample extension
with one server action and one panel, and test it in native and browser hosts.
