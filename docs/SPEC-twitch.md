# Spec: twitch

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `twitch` in the
[capability map](../CAPABILITY-MAP.md).

## Objective

Ship the first real platform integration as an executable Acheflow WASM extension:
connect an account/channel, receive/send chat, report current viewers and feed
application commands/polls. It is not a Twitch embedded video-player extension.

## Contracts and structure

Place guest source/manifest under `extensions/twitch/`. The core host owns generic
credential handles and capability-restricted HTTP/WebSocket
operations; the entire OAuth state machine, request mapping and event normalization
belong to the Rust guest under the [v2 contract](extension-contract.md).
Do not implement Twitch as a native-only shortcut around the extension runtime.
Connection controls/panels work through the common extension UI contracts.

Identify accounts, broadcasters and channels by Twitch ids. Use OAuth grants with
the scopes required by the selected installed/chat-client authorization model;
document app registration, redirect/device flow and scopes after verification in
the plan. Do not require users to paste tokens into chat or workspace JSON. Keep
credential handles on the server; UI sees identity/status and reconnect actions.

Use EventSub for incoming chat and Twitch API for sends. Manage subscription,
welcome/keepalive, reconnect handoff, revocation, cancellation and deduplication.
Translate message ids, users, roles, text/fragments and deletes/clears into the
interaction contract. Distinguish authorization loss, provider rate limiting,
offline status and transport failure. Outgoing send rejection/unknown outcome must
remain visible; do not claim delivery solely because a request was queued.

Fetch current viewers from the authorized Get Streams response `viewer_count`.
A successful channel lookup with no live stream marks it offline; an API failure
does not. Respect provider rate limits and retry guidance; polling and freshness
budgets are fixed in the plan. Reconnect must not duplicate a stream contributor.

Declare unique viewers unsupported unless a verified authorized Twitch interface
provides stream/channel uniques with a defined reporting window. Twitch extension
or game analytics cannot be relabeled as a broadcaster's stream uniques. If an
appropriate metric becomes available, emit it using the analytics optional unique
metric contract. Never derive it from chatters or concurrent counts.

## Acceptance and testing

- A real WASM guest connects and runs against deterministic Twitch HTTP/EventSub
  fixtures through restricted host APIs, including expiry, reconnect, rate limit,
  revoked grant, malformed data and duplicate message scenarios.
- A live account check proves chat receive/send, viewer sampling and a poll vote;
  report the credential-dependent evidence separately from fixture tests.
- Disconnect/disable closes connections and stops polling; token refresh and
  reauthorization update visible state without leaking credentials.
- Get Streams failure/offline/zero distinguish correctly; unique availability is
  explicit and carries the original scope/window when supported.

Commands: `cargo test -p acheflow-extension-api --locked`,
`cargo test -p acheflow-server --locked`, then shared checks. Add the exact selected
guest build/test commands to the implementation plan once the ABI/runtime is
chosen. Build with `python3 extensions/build.py`. Use typed, validated provider DTOs and the shared message/error style.

Always prove the integration through the actual extension boundary. YouTube,
VK Live and unrelated Twitch automation are later work. Native poll create/get/end
and periodic/final vote normalization are implemented in the guest.

## Official references checked on 2026-11-02

- [Chat & Chatbots](https://dev.twitch.tv/docs/chat/): EventSub and Twitch API.
- [EventSub WebSocket lifecycle](https://dev.twitch.tv/docs/eventsub/handling-websocket-events/).
- [Get Streams](https://dev.twitch.tv/docs/api/reference/#get-streams): current `viewer_count`.
- [Insights & Analytics](https://dev.twitch.tv/docs/insights): distinguish analytics
  for extensions/games from the requested stream audience metric.
