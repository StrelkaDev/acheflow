# Installed application forwarding

The forwarding address contract is implemented by
`acheflow_extension_api::forwarding::ForwardLink::parse_for`. OS activation,
manifest-declared delivery and hosted fallback pages are subsequent tasks; parsing
an address alone does not launch an application or authorize an extension action.

## Addresses

| Origin | Recipient |
| --- | --- |
| `https://client.acheflow.com` | Installed client |
| `https://server.acheflow.com` | Installed server |

Both use `/fwd/<extension_id>/<action>/...`. For example:

```text
https://client.acheflow.com/fwd/example/oauth/callback?code=opaque&state=opaque
```

`acheflow.com` remains the product website and `app.acheflow.com` identifies the
browser client. The forwarding origin is not a user's configured server endpoint.
Each receiver supplies its expected recipient to the parser; server/client links
cannot be substituted for each other. Canonical HTTPS authorities are required,
without explicit ports, user information or alternate host spellings.

The extension ID is the exact, case-sensitive manifest ID, validated by the same
rules as installation discovery. It is not a display name or a directory name.
The action is one nonempty ID segment of up to 64 ASCII letters, digits, underscores
or hyphens. Remaining path, query and fragment are opaque provider payloads.
URLs are bounded to 16 KiB. Noncanonical paths, malformed percent escapes, path
traversal and encoded path separators are rejected. ForwardLink's Debug output
omits the tail, query and fragment; applications must also avoid logging raw URLs.

## Installation ambiguity

Within each recipient's catalog, all installations sharing a valid manifest ID
are disabled. A third occurrence cannot restore an ambiguous ID. Errors identify
the conflicting directory paths or bundled installation slots. Unrelated valid
providers remain available, subject to the existing catalog capacity.

Directory scanning fails closed when the 64-entry scan limit is exceeded or entries
cannot all be read, rather than accepting a partial catalog with hidden duplicates.
Rejected duplicates cannot contribute browser authorization modules to a retained
installation.

## Delivery boundary

The parser is an addressing primitive, not an authenticated command API. Before
future delivery, the host must check the manifest, installed/enabled live instance
and its permitted action. OAuth completion additionally requires a pending local
transaction with matching state, workspace and instance lifetime. Provider flows,
scopes, token handling and callback interpretation remain under `extensions/`.

Implementation progress: [task list](../tasks/todo-desktop-links.md).
