# Step 06: Finalize Git Dependency

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` and update all consumers in lcp.

### Phase Context
Wave 3 — runs in parallel with step-05. Requires step-01 to be pushed to the
sourcehut remote before this step can succeed.

### This Step
Remove the temporary `[patch]` section from lcp's `Cargo.toml`, ensuring the
dependency resolves via the remote git URL. Update `Cargo.lock`. Clean up
`MASTER_PROGRESS.md`.

## Prerequisites
- Steps 02, 03, 04 all committed
- Step-01 commit pushed to sourcehut: `cd /home/ignacio/pr/its-classified && git push origin`
- The orchestrator must push step-01 before dispatching this step

## Files to Read Before Starting
- `Cargo.toml` (workspace root) — contains the `[patch]` section to remove
- `MASTER_PROGRESS.md` — update plan status

## Implementation

### Task 1: Push step-01 to remote (if not already done)

**This is an orchestrator responsibility** — the orchestrator should push
before dispatching this step. But verify:
```sh
cd /home/ignacio/pr/its-classified
git log --oneline -1 origin/HEAD 2>/dev/null  # check if remote has the rename commit
```

If the remote doesn't have the commit, push:
```sh
git push origin HEAD
```

### Task 2: Remove [patch] section from lcp Cargo.toml

Remove the entire block:
```toml
[patch."https://git.sr.ht/~mackenney/its-classified"]
doppel = { path = "/home/ignacio/pr/its-classified" }
```

### Task 3: Update Cargo.lock

```sh
cd /home/ignacio/pr/llm-cache-proxy
cargo update -p doppel
```

This regenerates the lock file to point at the remote git commit instead of the
local path.

### Task 4: Verify compilation against remote

```sh
cd /home/ignacio/pr/llm-cache-proxy
cargo build --workspace
cargo nextest run
```

### Task 5: Update MASTER_PROGRESS.md

Move `rename-to-doppel` from In Progress to Completed with the merge commit hash.

### Task 6: Commit

```sh
cd /home/ignacio/pr/llm-cache-proxy
git add -A
git commit -m "step-06: switch doppel to remote git dependency"
```

## Acceptance Criteria

- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -c '\[patch\]' Cargo.toml` outputs `0`
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo build --workspace` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'doppel' Cargo.lock` returns at least 1 match (dependency resolved)
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'rename-to-doppel' MASTER_PROGRESS.md` appears in Completed section

## Reviewer Instructions

You are reviewing Step 06 implementation. Verify:

1. `cd /home/ignacio/pr/llm-cache-proxy && grep '\[patch\]' Cargo.toml` — must return no matches
2. `cd /home/ignacio/pr/llm-cache-proxy && cargo build --workspace` — must exit 0
3. `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run` — must exit 0
4. `cd /home/ignacio/pr/llm-cache-proxy && grep 'doppel' Cargo.lock | head -3` — must show doppel dependency
5. Check `MASTER_PROGRESS.md` shows `rename-to-doppel` in Completed section

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit produced by this step.
