# Twitch guest

This client-side Rust/WASM guest owns Twitch authorization, validation, scopes,
EventSub lifecycle and Twitch API request mapping. Shared UI/server code stays
provider-neutral under the [extension contract](../../docs/extension-contract.md).

Enable the extension and use the manifest's Application setup instructions. Supply
your own application Client ID and exact callback; no application credentials are
inserted automatically. The current Connect flow uses an implicit grant and no
application secret. [Provider setup](../../docs/provider-setup.md) explains native
and browser registration. Access/refresh credentials remain local; supported native
macOS accounts use Keychain and browser tokens remain in memory.

The native provider runtime uses EventSub WebSocket for chat and lifecycle events,
Twitch API for sending, and Get Streams for current viewer samples. It preserves
provider event IDs, roles, receipt/echo correlation, bounded retries and cancellation.
Current viewers are not unique viewers. Unknown outgoing outcomes are not retried
automatically. Scope validation follows enabled chat-read, chat-send and
poll-management settings.

Local authorization does not start a continuous executor. The typed client bridge
supports chat/audience/deletion events and chat sends; native provider poll
instructions remain unfinished and are not advertised as live executor capability.
See [client execution](../../docs/client-executor.md). Qualify actual EventSub,
sends, expiry and provider permissions with your configured application/account.

Sources: [OAuth](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/),
[EventSub WebSocket](https://dev.twitch.tv/docs/eventsub/handling-websocket-events/),
[chat authentication](https://dev.twitch.tv/docs/chat/authenticating/),
[Get Streams](https://dev.twitch.tv/docs/api/reference/#get-streams).
