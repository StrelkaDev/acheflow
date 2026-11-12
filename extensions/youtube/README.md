# YouTube guest

This v2 guest uses the official YouTube Data/Live Streaming API. It discovers an
active broadcast for an authorized account, or uses the public `video_id` setting.
It receives live chat and deletion events, samples concurrent viewers every 30
seconds, and sends text with provider receipt IDs for host echo reconciliation.
Chat prefers the official gRPC `liveChatMessages.streamList` connection. If that
service returns HTTP 404/405/501 or gRPC UNIMPLEMENTED, it switches to the official
REST `liveChatMessages.list` API. REST requests follow `pollingIntervalMillis`,
preserve continuation tokens and obey rate-limit backoff. Pages request up to 200
messages with only the needed fields; raw JSON messages drain ten per WASM call
without increasing the fuel allowance. Stopped/replaced instances discard late
pages. Other permission and authentication failures remain visible.
Its protobuf schema and endpoint belong
to this extension; the native client supplies generic bounded HTTP/2 framing.
Fragments are at most 8 KiB, complete protobuf messages at most 512 KiB and batches
at most 2000 chat messages. Ten messages are normalized per guest call within the
unchanged WASM fuel allowance. Reading pauses while the current batch drains.
The continuation token advances only after that batch has been processed;
disconnects resume from it with bounded exponential backoff and message-ID
deduplication. Terminal/authorization errors require user attention; ended chat
closes its stream. Send timeouts are unknown and never automatically retried.

Chat reports connected only after the streaming transport confirms its open or
a REST chat request succeeds.
Periodic broadcast discovery does not clear a failed/reconnecting chat status.
Failures show HTTP/gRPC codes and a fixed local transport stage, without tokens
or provider response details. A successful outbound send uses a separate REST
request and does not prove the streaming connection is receiving messages.
Check **Read chat**, the selected live broadcast and this status when diagnosing
missing messages. After correcting a terminal error, use **Find live broadcasts**
to reopen the stream. Transient errors retry with backoff.

When no active chat is found, broadcast discovery backs off from 60 seconds to
five minutes. **Find live broadcasts** bypasses that idle delay. Once a live chat
is selected, broadcast/audience checks return to their normal 30-second interval.
Provider chat execution is independent of the local media stream's start/stop.

Enable the extension and expand **Application setup**. Its guest supplies links
to Google Auth Platform clients, API enablement and test-user configuration.
Native sign-in uses a Desktop OAuth client, its local application secret, the
active loopback callback and S256 PKCE. Browser sign-in uses a Web application
Client ID and the exact current JavaScript origin through Google Identity
Services; it never requests an application secret. Public Client ID and optional
video ID remain workspace-local settings. Requested scopes follow enabled
functions: `youtube.readonly` for reading, `youtube.force-ssl` for chat sending.

**Connect with YouTube** validates returned permissions and application identity.
Manual access-token entry is also available. Native refresh tokens are renewed
locally; committed native credentials use the system vault and browser credentials
last only for that session. No credentials are sent to the Acheflow server.
See [local authorization](../../docs/extension-authorization.md).

Native polls, donations, video output configuration and unique-viewer analytics
are not exposed by this version. API quotas still apply; streaming is not an
unlimited/free API bypass. Broadcast discovery and audience sampling remain
periodic. The stream executes in the native provider runtime library; the browser
account-setup UI remains supported, while an always-on/browser executor is still
unfinished; see [client execution](../../docs/client-executor.md). Qualify actual
account access, token renewal and the intended operations with your own application.

Sources: [streamList](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/streamList),
[official gRPC example and schema](https://developers.google.com/youtube/data/live/streaming-live-chat),
[chat insert](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/insert),
[REST chat list](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/list),
[videos](https://developers.google.com/youtube/data/reference/rest/v3/videos/list),
[desktop OAuth](https://developers.google.com/youtube/data/guides/auth/installed-apps).

Use [provider setup](../../docs/provider-setup.md) for user-owned application registration.
Do not include live application/account identifiers or callback data in examples.
