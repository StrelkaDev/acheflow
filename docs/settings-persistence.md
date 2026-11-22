# Settings persistence and restore

Local save,
applying a workspace to a server, and saving server listener settings are separate
operations. The server saves one workspace configuration; local catalogs and
existing device accounts remain on their devices; explicit server accounts are
separate. Authenticated clients follow the
server workspace automatically while retaining their local catalogs and drafts.

## Where settings live

| Data | Storage | Restore behavior |
|---|---|---|
| Native workspace catalog, selected workspace and client preferences | `~/.acheflow/workspaces.json`, overridden by `ACHEFLOW_WORKSPACE_FILE` | Loaded at client startup; edits auto-save through temporary-file replacement. Unix files are owner-only (`0600`). Parent directories are created. |
| Browser workspace catalog, selected workspace and client preferences | localStorage key `acheflow.workspaces.v1`, scoped to the browser profile and origin | Loaded on page startup; edits auto-save. A different scheme/host/port has a separate catalog. |
| Workspace server address, sources, stream keys and passphrases, scenes, outputs, audio, server logic `extensions`, local integration `client_extensions`, `client_capabilities` policy and `edit_revision` | Each workspace entry in the client catalog; also in workspace JSON exports | Kept independently for each workspace. Authenticated clients follow the server configuration; export/import also transfers independent local drafts. Camera and microphone IDs refer to devices on the configured server. |
| Background animation | Catalog `preferences.background_animation` | Shared across workspaces in this client catalog, restored after reload/restart. Old catalogs default to enabled. Excluded from workspace-only exports. |
| Server workspace configuration, capability policy and edit revision | `<config-stem>.workspace.json`, beside the selected server config | Apply and configuration commands save atomically before publishing edits. Restart restores the same identity/revision without starting inputs, outputs or integrations. Deactivation retains the saved configuration. Unix file mode is `0600`. |
| HTTP/RTMP bind settings and TLS certificate/key **paths** | `acheflow-server.json` relative to the server working directory; `--config` or `ACHEFLOW_CONFIG` selects another file | Loaded at server startup. Saving creates missing parent directories and atomically replaces the file. Listener changes need a restart. |
| Server provider bindings and their public settings | Workspace `server_integrations`, server workspace file and local authorized catalog replicas | Explicit Apply activates configured bindings. Restart restores them inactive; it does not transfer local accounts. |
| Server provider credentials | Server-ID/workspace OS-vault container; isolated provider/application/account slots | Separate from device-local accounts; no fallback. Renewal checkpoints atomically. Explicit connection revalidates and copies a committed local account. Replacement uses a fresh slot before retiring the previous one; disconnect removes the server slot. |
| Personal account ownership and public settings | Protected server access registry; private opaque vault slot references | Owned by the authenticated user, shared only by explicitly linked devices. Missing personal metadata in old files defaults empty. Excluded from workspace exports and snapshots. |
| Personal provider credentials | Same server-ID/workspace OS-vault container, disjoint user/provider/account slots | Explicit verified copy; no collector or channel grants. Versioned replacement preserves old credentials on save failure. Disconnect revokes ownership before cleanup. |
| Live client registrations, leases, receipts, chat/session state | Process memory | Not restored as live authority. Reconnect/re-register using a new generation; do not replay uncertain external sends. |
| Native macOS committed provider accounts/application secrets | Keychain, scoped to server/workspace/extension/application | Local account panels restore matching records; Disconnect removes them. No plaintext fallback. Other native vault adapters remain unfinished. |
| Browser account credentials and runtime opaque handles | Local process/session memory | Browser reload/disposal loses credentials. A native execution runtime must explicitly receive a matching local account. Only explicit trusted-server connection copies committed credentials to its vault. Values never enter workspace JSON or public MCP DTOs. |
| Desktop server startup/exit policy, ports and FFmpeg executable override | Catalog `preferences.server` | Shared across this catalog; excluded from workspace-only exports. The owned server configuration lives in sibling `server/`. |

A workspace import through **Add from file** creates a new workspace identity.
Update any executor endpoint's pinned `workspace_id` before using that imported copy.
RTMP keys are intentionally included in workspace files; platform/AI credentials
are not workspace settings. Removing obsolete manual token fields from an older
export is required before submitting it to the current server.

## Local save and server apply

**Saved locally** means the client catalog write succeeded. It does not mean that
new source/output/extension drafts have been applied to the server. Client integration fields and toggles auto-save locally in `client_extensions`.
Full **Apply workspace** / Start send a server projection that excludes this map.
Server logic `extensions` settings are included in Apply and their enable/disable
commands. An enable/disable command validates the installation and supplied public
settings, and loads the WASM guest before saving when enabling. The settings and
enablement share one edit revision. Validation, load or save failure preserves the
last saved settings and any running instance. A failure while handling guest
events after activation is reported in runtime status. The server rejects
nonempty client integration maps submitted directly.
Known installed providers migrate from the older shared `extensions` map when the
client loads/imports a workspace, without overwriting newer local preferences.
An enabled client card can authorize a local account; live server capabilities
still require an executor. Saved enablement does not open provider I/O on the server. Client preferences survive server-side merges
and conflict resolution. They are not a server-wide enable/disable switch.

Server scene-selection, extension-setting and capability-policy commands persist the
workspace independently of a connected client. Transient chats, polls, execution
receipts and media process state are not restored from this file.

In **Workspaces → Saved on the server**, **Save this workspace to server** reviews
and imports the local configuration explicitly. The local catalog and account
credentials remain intact. Import saves without activation; stop live outputs and
deactivate sources/extensions before replacing the server project. Connecting a
client alone never imports or switches a workspace.

`GET /api/workspace` returns server identity, saved revision, workspace summary
and a permission-filtered display projection.
The full configuration (including ingest/output keys) is available only with all
five Apply grants: workspace, scenes, media, extension configuration and execution.
`POST /api/workspace/import` requires the same grants and an `expected_revision`.
Malformed, stale or failed writes do not publish a replacement. A corrupt or
foreign-server file stops startup instead of silently resetting it. Keep the access
identity file alongside the workspace file when restoring the same server.

Native filesystem errors and browser localStorage/quota errors are shown in the
workspace manager. If initial storage loading fails, automatic writes are disabled
for that session so a corrupt/unreadable catalog is not silently replaced by defaults.
Keep/export the draft and repair local storage before restarting. Concurrent windows
still use the last local catalog write; there is no cross-window merge protocol.

## Conflicts and reconnects

Connected native/browser editors poll the authenticated workspace API every second
with bounded requests and backoff when unavailable. Saved revisions order workspace
updates across disconnects and server restarts. Responses from a previous endpoint,
identity or permission set cannot replace the current view. A delayed control GET
cannot override a newer command response; control requests time out after ten
seconds and release the busy state without automatically retrying writes.

A connected client attaches to the server workspace without importing, switching or
activating anything on the server. Its previous local workspace stays in the catalog.
The last authoritative configuration is saved as per-workspace catalog metadata,
separate from workspace-only exports. Reconnects use this base for a three-way
merge: unchanged local fields follow the server, unrelated local drafts survive,
and incompatible changes require explicit resolution. Lists (sources, scenes,
layers, outputs and capability grants) are atomic merge units: concurrent edits to
one list require a choice instead of guessing how to combine them. Local client
integration settings and account credentials are never copied from another client.

The sidebar distinguishes **Server workspace** from **Local draft**. Selecting or
creating a local workspace stops following the server until **Follow server
workspace** is chosen, or that draft is explicitly saved/applied. The server keeps
running during local draft editing. Following again preserves that draft in the
catalog before selecting the server workspace.

- **Use server workspace** selects the latest received configuration, preserving
  the local server address and matching workspace's client integration settings.
- **Keep local edits** preserves the draft and explicitly acknowledges the observed
  server revision. Apply remains a separate operation to replace configuration.

Conflicts appear above the main view, rather than only in the Extensions page.
Configuration commands wait for resolution; chat and independent controls remain
available within the device's grants. Apply/Start still enforce the server's edit
revision, including switching to another workspace. A server edit after resolution
can therefore cause another conflict.

Devices without all Apply grants receive a display projection with ingest/output
keys and transport URLs removed, no extension configuration, and no capability
policy. Scene/source details require studio, scene-control or interactive-view
rights; other authenticated viewers receive only the workspace identity/default
view. A display projection never serves as an editable merge base. Granting full
management access fetches the full authorized configuration before editing resumes.

## Server listener configuration

`GET /api/server/settings` reports active settings, saved settings, launch overrides
and `restart_required`. `PUT /api/server/settings` validates and saves listener/TLS
path settings, leaving the currently bound listeners unchanged. Invalid settings or
certificate/key files do not replace the saved JSON. The main UI has no listener
TLS/bind-address editor. Desktop Settings / Server manages the owned process,
local launch ports and connection; those controls do not hot-rebind active listeners.

Priority on startup: CLI arguments > environment overrides > saved JSON > defaults.
Remove a corresponding launch override to use a saved value. Saving workspace
settings never changes this server-wide file. Each separately operated server should
use its own config path and executor transport credentials.

## Verification

Regression tests cover first-contact/repeated-poll conflict protection, explicit
resolution, stale Apply rejection, native disk restore with two isolated workspaces,
policy/export roundtrips, catalog permissions/defaults, nested server settings paths,
fresh server-state loading and failed-write validation preserving the old file.
Two HTTP-backed UI clients cover shared updates and explicit conflicts; delayed
responses, server switching and command timeouts cover cancellation and recovery.

Local disk/browser restore and process restart need separate integration checks.
Verify public drafts/policy across reload, conflict resolution after disconnected
server edits, listener JSON after restart, and reapplication of the saved workspace.
Use isolated catalogs and local ports; keep live account data out of test reports.
