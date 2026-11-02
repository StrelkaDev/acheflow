# Spec: analytics

Status: implemented in the working tree; see [operational qualifications](controls.md). Module id: `analytics` in the
[capability map](../CAPABILITY-MAP.md). Viewer semantics incorporate the user's
2026-11-02 clarification: sum platform viewers; also fetch unique viewers whenever
the platform provides that separate metric.

## Objective

Display current viewer counts by connected stream, their arithmetic sum, session
peaks/history and chat activity. Collect platform-provided unique-viewer metrics
separately with their original scope and reporting window.

## Contracts and structure

Metric samples from guests use `acheflow-extension-api`; snapshots and queries use
`acheflow-media-api`; aggregation/history live in new server analytics modules;
UI presents counts, freshness and unavailable values consistently on both targets.

A metric sample identifies platform, connection, broadcaster/channel, provider
stream id if known, Acheflow session, metric kind, integer value, observed/fetched
times and availability. Unique-viewer samples additionally carry the provider's
scope, period start/end, timezone if required, and final/provisional status when
available. A delayed or cumulative unique count is not a concurrent count.

For current viewers, sum the latest fresh concurrent count for each distinct
platform/channel/stream. Multiple connections to the same stream must not double
count it. Aggregate connection samples into per-platform subtotals as well.
The user-facing total is an arithmetic sum, with no cross-platform deduplication.

Confirmed offline streams contribute zero. Unsupported, unauthorized, disconnected,
failed or stale samples are unavailable, not zero. Sum known fresh values and mark
the total partial with the missing contributors; if none are known, report no
current total. Preserve the last sample and its timestamp for inspection without
presenting it as current. Out-of-order samples cannot replace newer observations.

Expose current snapshots and bounded cursor/time-window queries. Record the
aggregate peak as the maximum simultaneous sampled total, not the sum of each
platform's independent peak. Peaks/history include coverage and freshness so a
partial sample is not presented as a complete observation. Define session reset,
sampling interval, freshness TTL and retention defaults in the implementation plan.
Initial history is scoped to the running session; durable cross-session reporting
is later work, and restart must not imply continuity of retained measurements.

Fetch unique viewers only through a supported, authorized provider interface.
Store and display them per provider stream/channel and period. Do not sum them
across platforms or time windows and do not infer them from chatters, messages,
OAuth users or concurrent counts. A real reported zero remains distinct from
missing data. Unique chat participants, if displayed, use that explicit label.
An adapter declares each metric supported/unsupported/permission-required;
availability can change after account authorization or a provider response.

## Acceptance and testing

- Fresh Twitch=100 and YouTube=50 produce a total of 150; no unique-person claim.
- Duplicate connections to one stream do not increase that sum.
- A platform unique count of 120 for a specified window appears separately; an
  unsupported provider has no fabricated value. Zero uniques display as zero.
- Stale/missing samples yield a partial total and visible status; confirmed offline
  is zero; delayed samples do not overwrite newer state.
- Simulated time proves TTL expiry, peak semantics, session reset, bounded history,
  counter overflow handling and chat-message deduplication for activity metrics.

Commands: `cargo test -p acheflow-media-api --locked`,
`cargo test -p acheflow-server --locked`, `cargo test -p acheflow-ui --locked`,
then shared checks. Use a controllable clock in aggregation tests and shared serde
style/error codes; no account is necessary for arithmetic/availability tests.

Always preserve provider meaning and periods. Do not promise a unique-viewer
endpoint for a platform until its integration verifies one.
