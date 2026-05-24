# Step 04: Replace all yield_now() with harness.wait_for_writes()

## Context

### Overall Objective
Clean up test infrastructure and proxy internals: deterministic cache-write synchronization, configurable channel capacity, and accurate field naming.

### Phase Context
Wave 2 — depends on step-03 which wired `JoinSet::spawn` into `handle()`. Now that background writes are tracked, tests can deterministically await completion instead of relying on `yield_now()`.

### This Step
Replace all 6 `tokio::task::yield_now().await` calls in spec tests with `harness.wait_for_writes().await`. Remove unused `yield_now` imports. This eliminates the scheduling-dependent race and makes cache-write assertions deterministic.

## Prerequisites
- Step 03 complete (JoinSet::spawn in handle(), wait_for_writes() available on TestHarness)

## Files to Read Before Starting
- `tests/spec/cache_miss.rs` — yield_now at lines 82, 109, 132
- `tests/spec/cache_hit.rs` — yield_now at lines 36, 100, 126
- `tests/common/harness.rs` — to confirm `wait_for_writes()` exists

## Implementation

### Task 1: Update `tests/spec/cache_miss.rs`
Replace each `tokio::task::yield_now().await;` with `harness.wait_for_writes().await;`:

- Line 82: `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`
- Line 109: `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`
- Line 132: `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`

Remove the `use tokio::task::yield_now;` import if present (or any inline `tokio::task::yield_now` references — these are called with full path, so there may not be an import to remove).

### Task 2: Update `tests/spec/cache_hit.rs`
Replace each `tokio::task::yield_now().await;` with `harness.wait_for_writes().await;`:

- Line 36 (inside `setup_hit_harness()`): `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`
- Line 100: `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`
- Line 126: `tokio::task::yield_now().await;` → `harness.wait_for_writes().await;`

Same import cleanup as Task 1.

### Task 3: Verify no remaining yield_now in test files
Run `grep -rn 'yield_now' tests/` — should return zero matches.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run` exits with code 0 (all tests pass deterministically)
- [ ] `cargo nextest run --test spec` exits with code 0 (spec invariants pass)
- [ ] `grep -rn 'yield_now' tests/` returns no matches
- [ ] `grep -rn 'wait_for_writes' tests/spec/cache_miss.rs` returns 3 matches
- [ ] `grep -rn 'wait_for_writes' tests/spec/cache_hit.rs` returns 3 matches

## Reviewer Instructions

You are reviewing Step 04 implementation. Verify:

1. Run `cargo nextest run` — must exit 0
2. Run `grep -rn 'yield_now' tests/` — must return no matches
3. Check `tests/spec/cache_miss.rs` — all 3 former yield_now sites now use `harness.wait_for_writes().await`
4. Check `tests/spec/cache_hit.rs` — all 3 former yield_now sites now use `harness.wait_for_writes().await`
5. No unused import warnings: `cargo clippy --tests` — no warnings about unused imports
6. Run the spec tests twice to confirm determinism: `cargo nextest run --test spec && cargo nextest run --test spec`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert HEAD` (single commit)
