# VK Video Live guest

Targets `live.vkvideo.ru` (formerly VK Play Live), using the official developer API
at `https://apidev.live.vkvideo.ru`. This is a separate service from `vk.com/video`.

The public `channel` setting is the channel slug, without the URL. The guest reads
chat every five seconds, deduplicates provider IDs, normalizes text/link/smile/
mention parts and moderator/owner roles, and samples concurrent viewers every
30 seconds. Private messages are excluded. History pages are decoded in bounded
chunks. The history endpoint exposes a window of at most 200 messages, rather
than a continuation cursor; a disjoint full window produces a visible gap warning.
Receiving chat via this polling endpoint cannot guarantee lossless high-rate chat.

Enable the extension, enter the channel slug and expand **Application setup**.
The guest supplies developer-portal/authorization links, native/browser application
instructions, the exact active redirect with **Copy**, and its public Client ID
field. This DevAPI implicit flow uses no application secret. VK ID may handle
portal sign-in but does not replace the separate DevAPI token. **Connect with VK
Video Live** requests account consent; manual access-token entry remains available.
Credentials stay local to the client: native committed accounts use the system
vault and browser tokens last only for the session. No automatic refresh is
claimed for this flow. Reconnect after expiry. See
[local authorization](../../docs/extension-authorization.md).

Channels advertise no outbound capabilities. The official send endpoint returns
an empty receipt, which cannot reliably link an outgoing host message to its
provider echo. Sending requires resolving that contract limitation first. Native
polls and automatic RTMP configuration are not exposed.

Sources: [API entry point](https://dev.live.vkvideo.ru/docs/index),
[authorization](https://dev.live.vkvideo.ru/docs/main/authorization),
[chat](https://dev.live.vkvideo.ru/docs/method/chat),
[channel](https://dev.live.vkvideo.ru/docs/method/channel).

Use [provider setup](../../docs/provider-setup.md) for user-owned application registration.
Do not include live application/account identifiers or callback data in examples.
