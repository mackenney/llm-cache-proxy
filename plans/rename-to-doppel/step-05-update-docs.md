# Step 05: Update Documentation

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` and update all consumers in lcp.

### Phase Context
Wave 3 — runs after all code changes (steps 02-04) are complete. Pure
documentation updates, no behavioral changes. Runs in parallel with step-06.

### This Step
Update all SPEC.md files in the lcp repo, AGENTS.md, and any remaining inline
doc comments that still reference old names. Also verify/update docs in the
its-classified (doppel) repo that step-01 may have missed.

## Prerequisites
- Steps 02, 03, 04 all committed

## Files to Read Before Starting

### lcp repo (`/home/ignacio/pr/llm-cache-proxy`)
- `SPEC.md` — references `its-classified`, `[extensions.scrub]`, `scrub/unscrub`, `patterns_file`, `Tier 2`
- `crates/lcp-server/SPEC.md` — references `ScrubExt`, `SseUnscrubStream`, `unscrub_stream`, `scrub/unscrub`
- `AGENTS.md` — may reference `its-classified`
- `crates/lcp-server/src/ext/doppel.rs` — verify module-level `//!` doc is updated
- `crates/lcp-server/src/ext/sse_restore.rs` — verify comments are updated

### doppel repo (`/home/ignacio/pr/its-classified`)
- `SPEC.md` — verify all old names replaced (should be done in step-01, catch stragglers)
- `README.md` — verify all old names replaced
- `AGENTS.md` — verify test commands updated

## Implementation

### Task 1: lcp SPEC.md

Run a comprehensive search first:
```sh
cd /home/ignacio/pr/llm-cache-proxy
grep -nE "its.classified|its_classified|\[extensions\.scrub\]|patterns_file|scrub/unscrub|unscrub|ScrubExt|SseUnscrub|PatternsFile|Tier.1|Tier.2" SPEC.md
```

Key replacements:
- `its-classified` → `doppel`
- `[extensions.scrub]` → `[extensions.doppel]`
- `scrub/unscrub extension` → `doppel extension`
- `scrub extension` → `doppel extension`
- `patterns_file` (config key) → `secrets_file`
- `Tier 2 secrets` → `registered secrets` (where appropriate)
- `Tier 1` → `built-in patterns` (where appropriate, keep where it refers to the on-disk format)
- `its-classified init <path>` → `doppel init <path>`
- `its-classified register --patterns <path>` → `doppel secret add <value>`
- `§SSE-Aware Unscrubbing` section header → `§SSE-Aware Restore`

### Task 2: crates/lcp-server/SPEC.md

```sh
cd /home/ignacio/pr/llm-cache-proxy
grep -nE "ScrubExt|SseUnscrub|unscrub_stream|scrub/unscrub|unscrub|scrub" crates/lcp-server/SPEC.md
```

Key replacements:
- `ScrubExt` → `DoppelExt`
- `SseUnscrubStream` → `SseRestoreStream`
- `unscrub_stream` → `restore_stream`
- `scrub/unscrub` → `swap/restore`
- `§SSE-Aware Unscrubbing` → `§SSE-Aware Restore`
- `unscrub pipeline` → `restore pipeline`
- `scrubbing` → `swapping` (Phase 2); `unscrubbing` → `restoring` (Phase 3)

### Task 3: Verify doppel repo docs (catch step-01 stragglers)

```sh
cd /home/ignacio/pr/its-classified
grep -rE "its.classified|its_classified|ScrubResult|UnscrubStream|PatternsFile|Tier1Entry|Tier2Entry|RegistrationOptions|RegistrationError|ITS_CLASSIFIED_KEY" SPEC.md README.md AGENTS.md --include="*.md"
```

If any matches found, fix them. These should have been caught in step-01 but
SPEC.md is large and some references may have been missed.

### Task 4: lcp AGENTS.md

Check for `its-classified` references:
```sh
grep -n "its.classified" /home/ignacio/pr/llm-cache-proxy/AGENTS.md
```
Update any found.

### Task 5: Scan for remaining old names in lcp source comments

```sh
cd /home/ignacio/pr/llm-cache-proxy
grep -rn "its.classified\|its_classified\|ScrubExt\|SseUnscrub\|PatternsFile\|Tier1Entry\|Tier2Entry" \
  crates/lcp-server/src/ crates/lcp/src/ --include="*.rs"
```

Update any matches found in doc comments (`///`, `//!`). Ignore matches in
serde alias strings (those are intentional for backward compat).

### Task 6: Build and test both repos
```sh
cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run
cd /home/ignacio/pr/its-classified && cargo nextest run --workspace
```

### Task 7: Commit (lcp repo only; doppel fixes go in a separate commit if needed)
```sh
cd /home/ignacio/pr/llm-cache-proxy
git add -A
git commit -m "step-05: update SPEC.md and doc comments for doppel rename"
```

If doppel repo fixes were needed:
```sh
cd /home/ignacio/pr/its-classified
git add -A
git commit --amend --no-edit  # amend onto step-01 commit
```

## Acceptance Criteria

- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -riE "its.classified|ScrubExt|SseUnscrubStream|unscrub_stream|PatternsFile|Tier1Entry|Tier2Entry|RegistrationOptions|RegistrationError" SPEC.md crates/lcp-server/SPEC.md` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -E "\[extensions\.scrub\]|patterns_file" SPEC.md` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -riE "its.classified|ScrubExt|SseUnscrubStream" crates/lcp-server/src/ crates/lcp/src/ --include="*.rs"` returns no matches (excluding serde alias strings)
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run` exits 0
- [ ] `cd /home/ignacio/pr/its-classified && cargo nextest run --workspace` exits 0
- [ ] `cd /home/ignacio/pr/its-classified && grep -riE "its.classified|its_classified|ScrubResult|UnscrubStream|PatternsFile|ITS_CLASSIFIED_KEY" SPEC.md README.md AGENTS.md` returns no matches

## Reviewer Instructions

You are reviewing Step 05 implementation. Verify:

1. `cd /home/ignacio/pr/llm-cache-proxy && grep -riE "its.classified|ScrubExt|SseUnscrubStream|PatternsFile|Tier1Entry|RegistrationOptions" SPEC.md crates/lcp-server/SPEC.md` — must return no matches
2. `cd /home/ignacio/pr/llm-cache-proxy && grep -E "\[extensions\.scrub\]|patterns_file" SPEC.md` — must return no matches
3. `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run` — must exit 0
4. `cd /home/ignacio/pr/its-classified && cargo nextest run --workspace` — must exit 0
5. Spot-check `crates/lcp-server/SPEC.md` `§SSE-Aware Restore` section — must use `DoppelExt`, `SseRestoreStream`, `restore_stream`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the commit produced by this step.
