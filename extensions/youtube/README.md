# YouTube guest

This v2 guest uses the official YouTube Data/Live Streaming API. It discovers an
active broadcast for an authorized account, or uses the public `video_id` setting.
It receives live chat and deletion events, samples concurrent viewers every 30
seconds, and sends text with provider receipt IDs for host echo reconciliation.
Chat pagination respects `pollingIntervalMillis`; a minimum 200-message API page
is decoded incrementally inside the existing WASM fuel limit. Send timeouts are
reported as unknown and are never automatically retried.

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
are not exposed by this version. API quota limits still apply to polling.

Sources: [chat list](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/list),
[chat insert](https://developers.google.com/youtube/data/live/reference/rest/v3/liveChatMessages/insert),
[videos](https://developers.google.com/youtube/data/reference/rest/v3/videos/list),
[desktop OAuth](https://developers.google.com/youtube/data/guides/auth/installed-apps).
