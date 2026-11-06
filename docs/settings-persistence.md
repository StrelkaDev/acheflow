# Settings persistence and restore

Local save,
applying a workspace to a server, and saving server listener settings are separate
operations. The logic server has no durable workspace catalog or platform credentials.

## Where settings live

| Data | Storage | Restore behavior |
|---|---|---|
| Native workspace catalog, selected workspace and client preferences | `~/.acheflow/workspaces.json`, overridden by `ACHEFLOW_WORKSPACE_FILE` | Loaded at client startup; edits auto-save through temporary-file replacement. Unix files are owner-only (`0600`). Parent directories are created. |
| Browser workspace catalog, selected workspace and client preferences | localStorage key `acheflow.workspaces.v1`, scoped to the browser profile and origin | Loaded on page startup; edits auto-save. A different scheme/host/port has a separate catalog. |
| Workspace server address, RTMP inputs/keys, scenes, outputs, audio, server logic `extensions`, local integration `client_extensions`, `client_capabilities` policy and `edit_revision` | Each workspace entry in the client catalog; also in workspace JSON exports | Kept independently for each workspace. Export/import is the transfer mechanism between browser and native clients. |
| Background animation | Catalog `preferences.background_animation` | Shared across workspaces in this client catalog, restored after reload/restart. Old catalogs default to enabled. Excluded from workspace-only exports. |
| Active workspace and its public capability policy on the server | Server process memory | Apply sends the selected local workspace to its server. Server restart/deactivation clears the active workspace; apply the saved client workspace again. |
| HTTP/RTMP bind settings and TLS certificate/key **paths** | `acheflow-server.json` relative to the server working directory; `--config` or `ACHEFLOW_CONFIG` selects another file | Loaded at server startup. Saving creates missing parent directories and atomically replaces the file. Listener changes need a restart. |
| Live client registrations, leases, receipts, chat/session state | Process memory | Not restored as live authority. Reconnect/re-register using a new generation; do not replay uncertain external sends. |
| Native macOS committed provider accounts/application secrets | Keychain, scoped to server/workspace/extension/application | Local account panels restore matching records; Disconnect removes them. No plaintext fallback. Other native vault adapters remain unfinished. |
| Browser account credentials and runtime opaque handles | Local process/session memory | Browser reload/disposal loses credentials. A native execution runtime must explicitly receive a matching local account. Values never enter server state, workspace JSON or public MCP DTOs. |
| Desktop server startup/exit policy and ports | Catalog `preferences.server` | Shared across this catalog; excluded from workspace-only exports. The owned server configuration lives in sibling `server/`. |

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
commands. The server rejects nonempty client integration maps submitted directly.
Known installed providers migrate from the older shared `extensions` map when the
client loads/imports a workspace, without overwriting newer local preferences.
An enabled client card can authorize a local account; live server capabilities
still require an executor. Saved enablement does not open provider I/O on the server. Client preferences survive server-side merges
and conflict resolution. They are not a server-wide enable/disable switch.

Server `set_client_capabilities` and extension control commands update the active
workspace in memory. Connected clients merge public server changes into their local
workspace and auto-save the result. An MCP-only change has no durable workspace
backup until a client reconciles/saves it or an operator stores the public configuration
separately. There is no automatic server workspace database or catalog replication.

Native filesystem errors and browser localStorage/quota errors are shown in the
workspace manager. If initial storage loading fails, automatic writes are disabled
for that session so a corrupt/unreadable catalog is not silently replaced by defaults.
Keep/export the draft and repair local storage before restarting. Concurrent windows
still use the last local catalog write; there is no cross-window merge protocol.

## Conflicts and reconnects

The control snapshot includes public extension settings, capability grants, scene
state and revisions. Normal polling performs a three-way merge against the previous
snapshot: unrelated local drafts survive, and conflicting values are preserved as
local drafts rather than silently overwritten.

On the first snapshot after reconnect, a stale local revision with different public
extension/policy/3D settings raises a conflict. The client does **not** advance its
workspace revision while that conflict remains unresolved. Repeated polling and
Stop/output-control responses do not acknowledge an unresolved settings change.

- **Use server settings** copies the latest observed public extension config,
  capability policy and matching scenes' 3D state into the local workspace, then
  acknowledges that revision and saves locally. Other workspace drafts remain local.
- **Keep local edits** preserves the local draft and explicitly acknowledges the
  observed server revision. It does not send an Apply; apply the draft separately
  when you intend to replace the server configuration.

The server rejects stale Apply requests that would replace its extension settings,
capability policy or 3D scene state. Revision protection covers these control-owned
fields; it is not a full distributed merge for every RTMP/output/name field. Changes
arriving after conflict resolution may require another reconciliation.

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

Local disk/browser restore and process restart need separate integration checks.
Verify public drafts/policy across reload, conflict resolution after disconnected
server edits, listener JSON after restart, and reapplication of the saved workspace.
Use isolated catalogs and local ports; keep live account data out of test reports.
