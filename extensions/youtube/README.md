# YouTube guest

This v2 guest uses the official YouTube Data/Live Streaming API. It discovers an
active broadcast for an authorized account, or uses the public `video_id` setting.
It receives live chat and deletion events, samples concurrent viewers every 30
seconds, and sends text with provider receipt IDs for host echo reconciliation.
Chat pagination respects `pollingIntervalMillis`; a minimum 200-message API page
is decoded incrementally inside the existing WASM fuel limit. Send timeouts are
reported as unknown and are never automatically retried.

The guest expects the opaque `access_token` secret with the
`https://www.googleapis.com/auth/youtube.force-ssl` scope. Enable the extension,
enter the video ID if needed, paste an OAuth access token into its masked Access
token field, and choose Save on server. Obtain the token through Google's OAuth
flow for your registered application; an API key or client secret is not an access
token. Saving starts a fresh guest with the latest public settings. The field is
write-only and remains empty after saving; the server returns configured status
only. Credentials remain in server memory until restart, independently for every
workspace and provider. Remove deletes the token and restarts the enabled guest.
Authorization expiry fails the instance and removes live channels. Replace the
token to reconnect. No automatic token refresh or built-in OAuth login is claimed.

Native polls, donations, video output configuration and unique-viewer analytics
are not exposed by this version. API quota limits still apply to polling.

Sources: [chat list](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/list),
[chat insert](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/insert),
[videos](https://developers.google.com/youtube/data/reference/rest/v3/videos/list),
[desktop OAuth](https://developers.google.com/youtube/data/guides/auth/installed-apps).
