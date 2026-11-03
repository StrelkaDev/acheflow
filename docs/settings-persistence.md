# Settings persistence and restore

Verified against the client/server implementation on 2026-11-03. Local save,
applying a workspace to a server, and saving server listener settings are separate
operations. The logic server has no durable workspace catalog or platform credentials.

## Where settings live

| Data | Storage | Restore behavior |
|---|---|---|
| Native workspace catalog, selected workspace and client preferences | `~/.acheflow/workspaces.json`, overridden by `ACHEFLOW_WORKSPACE_FILE` | Loaded at client startup; edits auto-save through temporary-file replacement. Unix files are owner-only (`0600`). Parent directories are created. |
| Browser workspace catalog, selected workspace and client preferences | localStorage key `acheflow.workspaces.v1`, scoped to the browser profile and origin | Loaded on page startup; edits auto-save. A different scheme/host/port has a separate catalog. |
| Workspace server address, RTMP inputs/keys, scenes, outputs, audio, extension enablement/public config, `client_capabilities` policy and `edit_revision` | Each workspace entry in the client catalog; also in workspace JSON exports | Kept independently for each workspace. Export/import is the transfer mechanism between browser and native clients. |
| Background animation | Catalog `preferences.background_animation` | Shared across workspaces in this client catalog, restored after reload/restart. Old catalogs default to enabled. Excluded from workspace-only exports. |
| Active workspace and its public capability policy on the server | Server process memory | Apply sends the selected local workspace to its server. Server restart/deactivation clears the active workspace; apply the saved client workspace again. |
| HTTP/RTMP bind settings and TLS certificate/key **paths** | `acheflow-server.json` relative to the server working directory; `--config` or `ACHEFLOW_CONFIG` selects another file | Loaded at server startup. Saving creates missing parent directories and atomically replaces the file. Listener changes need a restart. |
| Live client registrations, leases, receipts, chat/session state | Process memory | Not restored as live authority. Reconnect/re-register using a new generation; do not replay uncertain external sends. |
| Captured provider OAuth credentials in `acheflow-client` | Client runtime memory only | No durable vault yet. Disposing/restarting the executor loses these credentials. They are excluded from server state, client workspace JSON and public MCP DTOs. |

A workspace import through **Add from file** creates a new workspace identity.
Update any executor endpoint's pinned `workspace_id` before using that imported copy.
RTMP keys are intentionally included in workspace files; platform/AI credentials
are not workspace settings. Removing obsolete manual token fields from an older
export is required before submitting it to the current server.

## Local save and server apply

**Saved locally** means the client catalog write succeeded. It does not mean that
new source/output/extension drafts have been applied to the server. Public extension
fields auto-save locally; full **Apply workspace** sends the workspace, while an
extension enable/disable command also sends that extension's current public config.
A client provider still needs a registered executor; saved enablement does not open
a platform connection on the server.

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
settings editor; its Settings tab controls client appearance.

Priority on startup: CLI arguments > environment overrides > saved JSON > defaults.
Remove a corresponding launch override to use a saved value. Saving workspace
settings never changes this server-wide file. Each separately operated server should
use its own config path and executor transport credentials.

## Verification

Regression tests cover first-contact/repeated-poll conflict protection, explicit
resolution, stale Apply rejection, native disk restore with two isolated workspaces,
policy/export roundtrips, catalog permissions/defaults, nested server settings paths,
fresh server-state loading and failed-write validation preserving the old file.

A separate loopback server and a real Chrome/WebGPU client additionally verified:
background animation surviving reload; an extension public draft surviving reload;
a server policy change while the client was disconnected producing a visible
conflict; choosing the server version and reloading; listener JSON being loaded by
a restarted server without port flags; and the saved browser workspace restoring
public extension config **and** capability policy through Apply after that restart.
The test tab/server were stopped afterward; the user's running server was untouched.
Live platform OAuth and durable executor credentials remain separate unfinished work.
