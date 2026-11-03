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

The guest expects an opaque `access_token` for a user or application. Register an
application in the VK Video Live developer console and obtain its token using the
documented OAuth flow. Enable the extension, enter the channel slug, paste the
token into its masked Access token field, then choose Save on server. This is a
write-only credential field: snapshots and exported workspaces never contain the
value. Credentials remain in server memory until restart and are separate for
each workspace and provider. Remove deletes the token. Saving or removing a token
restarts an enabled guest. No built-in OAuth login or automatic refresh is claimed.
Expired authorization fails the instance and removes live channels; save a new
token to reconnect.

Channels advertise no outbound capabilities. The official send endpoint returns
an empty receipt, which cannot reliably link an outgoing host message to its
provider echo. Sending requires resolving that contract limitation first. Native
polls and automatic RTMP configuration are not exposed.

Sources: [API entry point](https://dev.live.vkvideo.ru/docs/index),
[authorization](https://dev.live.vkvideo.ru/docs/main/authorization),
[chat](https://dev.live.vkvideo.ru/docs/method/chat),
[channel](https://dev.live.vkvideo.ru/docs/method/channel).
