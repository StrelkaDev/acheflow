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
