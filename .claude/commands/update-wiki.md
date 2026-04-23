Sync the crabsim wiki with recent source code changes.

## Step 1 — Find changed Rust source files

Run both commands and combine results:

```
git diff HEAD --name-only -- src/
git diff HEAD~1 HEAD --name-only -- src/
```

Keep only `.rs` files. Deduplicate.

If the combined set is empty, report "Wiki is up to date — no Rust source changes detected." and stop.

## Step 2 — Inspect the diffs

For each changed file run the relevant diff:
- Uncommitted: `git diff HEAD -- <file>`
- Last commit: `git diff HEAD~1 HEAD -- <file>`

Read each diff carefully: renamed fields, new or removed methods, changed function signatures, altered behaviour, new types.

## Step 3 — Find affected wiki pages

List all `.md` files under `wiki/` excluding `wiki/_ai/` and `wiki/_template/`.
Read each page and check its "Code references" section for mentions of the changed source files.
Collect every page that references at least one changed file.

## Step 4 — Update wiki pages

For each affected page:

1. Read the full page and the current source file(s) it references.
2. Identify every inaccuracy introduced by the changes.
3. Update only the inaccurate parts — preserve structure, tone, and YAML front-matter.
4. Immediately update the matching `wiki/_ai/<section>/<filename>.md` compact summary with the same corrections.

## Step 5 — Regenerate all.md if needed

If any pages were updated, regenerate `wiki/_ai/all.md`.
Re-read all compact summaries under `wiki/_ai/` in order: concepts → architecture → guides → reference.
Rebuild `all.md` to reflect the current state of all summaries.

## Step 6 — Report

Print:
- Source files that had changes (and whether from uncommitted work or last commit)
- Wiki pages updated, with a one-line description of what changed in each
- Wiki pages checked and found accurate
