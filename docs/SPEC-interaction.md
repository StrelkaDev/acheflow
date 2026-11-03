# Spec: interaction

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `interaction` in the
[capability map](../CAPABILITY-MAP.md).

## Objective

Provide one application chat across connected channels, explicit outgoing routing,
chat commands and application-owned polls. A configured command can request a
permitted scene action through the same handlers used by the editor and MCP.

## Contracts and structure

Shared chat/event types belong in `acheflow-extension-api`; application snapshots
and command results in `acheflow-media-api`; server state in new interaction
modules; native/web chat and poll controls in `acheflow-ui/src`.

Chat belongs to Live / Studio rather than the application navigation. A running
live session places it in a shared right sidebar across all tabs, retaining the
draft and outgoing channel. Narrow layouts stack that sidebar below the page.
Polls and scene interactions belong to a separate Live / Interactive tab.

A chat message carries connection/platform/channel, provider message id, provider
user id, display name, server-validated roles, text, optional typed fragments,
occurrence and receipt times. Stable identity is the namespaced provider id, never
the display name. Handle message deletion/clear events where supplied. Unsupported
features are declared by the connection. Render external text as text.

Expose bounded cursor-based message reads and snapshots, incoming event delivery,
send-message commands, command registration/update/enable/disable and poll
create/read/close/cancel. Sending explicitly selects All channels (empty target list) or particular
registered channel keys. Fanout is limited to enabled capable channel owners. Return queued, sent, rejected or unknown
outcomes with a correlation id; incoming echo must not create duplicate messages.

Command definitions contain a name, typed arguments, allowed platform/channel,
roles, cooldown and a registered host/extension action. Reject unknown actions and
invalid arguments. Command processing uses provider user identity and validated
roles, enforces cooldowns atomically and deduplicates replayed events. No shell,
arbitrary code or unrestricted MCP invocation is available to a chat command.

Polls are application-owned aggregates of native provider polls. Creation routes
commands to selected poll-capable extensions; each retains its external poll ID
and publishes initial, periodic and final votes through the v2 event API. One
active aggregate per channel; overlapping targets conflict. Closure requests final
results and preserves per-target pending/closing/completed/failed state. A provider
without poll support is unavailable rather than silently using chat voting.
Donations are normalized integer minor units/currency/decimal scale, deduplicated
by extension + receipt ID. Direct and extension-declared interactions register
through the same host domain API and validated scene-action handler.

Owner/operator actions require application authorization; viewer commands receive
only their configured permissions. Chat commands use a deterministic parser; native votes arrive as provider poll
updates and never execute host commands. Retention,
text length, batch and cooldown defaults are fixed before implementation.

## Acceptance and testing

- Receive and send Twitch chat, with channel selection, status and visible errors
  in native and web UI. Show deletes/clears supported by the adapter.
- Test duplicate/reordered delivery, reconnects, bounded history and cursor gaps.
- Test role spoofing, concurrent cooldown checks, disabled commands and invalid
  arguments against a real command dispatcher.
- Test two real WASM providers, all/single chat routing, receipts and late echoes,
  duplicate/periodic/final poll updates and partial failures; only accepted
  interactions reach scene services.

Commands: `cargo test -p acheflow-extension-api --locked`,
`cargo test -p acheflow-server --locked`, `cargo test -p acheflow-ui --locked`,
then the shared implementation checks. Use shared typed serde style/error codes.

Always route side effects through granted application actions. See [the v2 contract](extension-contract.md). Additional moderation tools beyond
receiving deletion/clear events require later specs.
