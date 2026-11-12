# Documentation

Acheflow is developing as a studio for content creators, with live video production
and streaming as its current foundation. These guides describe the current working
tree. [The roadmap](roadmap.md) separates unfinished work from available behavior.

## Use Acheflow

- [Getting started](getting-started.md): prerequisites, desktop/server/browser launch
  modes, custom ports, remote connections and the first broadcast.
- [Desktop installation](desktop-installation.md): macOS apps, link registration,
  packages, data locations and FFmpeg.
- [Live production](live-pipeline.md): source setup, scene editing, CSS/WGSL,
  output profiles, status and media APIs.
- [Audio](audio-capabilities.md): independent tracks, scene buses, mixer controls,
  routing and current limits.
- [Interaction, audience, 3D and MCP](controls.md): available controls and the
  boundary between local authorization and connected executors.
- [Provider setup](provider-setup.md) and [local authorization](extension-authorization.md):
  user-owned applications, callbacks, permissions and local credential lifetime.
- [Settings and restore](settings-persistence.md): catalogs, Apply, conflict handling
  and what survives a restart.

## Operate an installation

- [Owned local server](local-server.md): readiness, process ownership and exit policy.
- [FFmpeg](ffmpeg.md): discovery, codecs, explicit encoder selection and media tests.
- [Forwarding](forwarding.md): recipient grammar, private IPC and lifecycle checks.
- [Native link installation](native-links-installation.md): OS registration, shared-domain
  fallback, association metadata and external deployment requirements.

## Develop Acheflow

- [Architecture](architecture.md): packages, media/control flows and ownership.
- [Development](development.md): build commands, checks and documentation rules.
- [UI integration](ui.md): shared native/browser rendering, input and performance.
- [Extension contract v2](extension-contract.md): governing provider-neutral boundary.
- [Extension protocol reference](extensions.md) and [authoring guide](../extensions/README.md).
- [Client executor](client-executor.md): library integration, grants and supported operations.
- [ADR-002](decisions/002-client-execution-and-workspace-capabilities.md): current
  credential ownership; [ADR-001](decisions/001-provider-authentication.md) records
  the superseded decision.

Documentation contains reusable configuration examples, not live registration
records or transcripts of test sessions. Obtain installed application versions,
current test results and environment-specific identifiers from the owning tools.
