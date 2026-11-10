# Working efficiently

Scale the workflow to the requested action. A request to commit or merge completed
work is a Git task; do not restart implementation or a full feature workflow.

## Commits and merges

- Commit or merge only when the user explicitly requests it.
- Default to one commit for the completed task, covering its related files with a
  descriptive message. Do not split by file or hunk unless requested or needed to
  separate unrelated work. Never rewrite existing commits without authorization.
- Inspect status and the relevant diff once. Reuse checks from this chat when they
  passed on the same code, with unchanged dependencies and test environment.
  Rerun affected checks after changes, failures, or material uncertainty. Complete
  required project checks; do not weaken the quality bar to save tokens.
- Do not edit plans or task files solely to record commit/merge permission.
- Verify the final branch, commit and status together. Refresh Repowise once after
  the final Git operation when refreshed context is needed.

## Context and tools

- Load only skills applicable to the current action. Read a skill once per chat
  and reuse it; do not follow its references or adjacent skills without a concrete
  need. A Git task does not itself require a new spec, plan, ADR or runtime review.
- Use Repowise for code understanding and impact analysis. Reuse a current review;
  request additional risk/history analysis only to resolve a specific uncertainty.
- Start with targeted searches and file sections. Summarize large logs locally;
  avoid dumping full histories, configuration files or generated output.
- Batch independent reads and keep outputs bounded to avoid truncation and rereads.
- Discover tools by exact service/action and print only matching names or the one
  needed schema. Never dump the full tool catalog or raw session transcripts.
- Use focused UI state, DOM values or existing tests to check numeric/behavioral
  changes. Capture screenshots for visual decisions and a final visual check;
  repeat them only when a new change or unresolved failure requires it.
- Keep large build/test logs in temporary files and return the result, failing
  cases and relevant excerpts. Expand output when needed to diagnose a failure.
- Stop after the requested result and its necessary verification are complete.
  Keep progress and final reports brief and factual.

## Extension architecture

- Follow [the v2 extension contract](docs/extension-contract.md) for every extension.
- Keep provider authentication, URLs, protocol mappings and subscriptions under
  `extensions/`. Production server/UI code must stay provider-neutral.
- Register settings, channels and capabilities through the manifest/domain API.
  Adding a provider must not require editing the server or shared UI.
- After changing guest/SDK source, run `python3 extensions/build.py` before host
  builds or tests; keep source, manifests and local WASM artifacts matched.
  WASM compiled from repository source is generated output: never track or commit
  it, including test fixtures. Generate it during setup, CI and packaging.
- Publish only from enabled live instances. Test two independent providers and
  lifecycle/cancellation boundaries; use real WASM guests, not native substitutes.


## Documentation

- Keep README focused on Acheflow as a content-creation studio, with streaming
  capabilities described at their actual implementation level.
- Use `docs/README.md` as the guide index and `docs/roadmap.md` for unfinished work.
  Update current guides instead of accumulating completed plans or session logs.
- Use placeholders in examples. Never copy real application/client/account/project,
  workspace/session or process identifiers, user emails, callback payloads, tokens
  or machine-specific paths from verification runs into documentation.
- Check local links, launch commands and identifier leakage after documentation
  edits. Keep live test evidence in private or ignored temporary artifacts.
