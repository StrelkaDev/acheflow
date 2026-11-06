# ADR-001: Separate bot and personal account credentials

## Status

Superseded by [ADR-002](002-client-execution-and-workspace-capabilities.md).
This record preserves the rationale for the earlier decision; use ADR-002 and the
[extension contract](../extension-contract.md) for current implementation rules.

## Context

Independently operated Acheflow installations need control of their own provider
applications, accounts and external AI services. A common embedded token or a
required central service would prevent that independence. Personal authorization
must not disclose account credentials to arbitrary logic servers.

## Historical decision

The earlier design separated a server-operated bot identity from personal client
authorization. It kept personal sends and personal tokens on the client, while
allowing a server-private bot credential store for automated provider work.

## Why it was superseded

The current boundary moves bot credentials and provider execution to clients too.
The logic server now receives bounded normalized events, applies public policy and
returns typed instructions. An optional independently operated or managed executor
acts as a client; it does not restore a credential store inside the logic server.

## Retained principles

Operators choose their own applications/accounts. Private credentials are never
embedded in artifacts or public examples. Workspace switching, disconnect and
revocation must not select another sender or replay uncertain external writes.
Managed AI/bot services remain optional and require explicit configuration.
