# Step 06: Add E2E Test for Default Body Limit

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 2 adds tests. This step adds an e2e test verifying the default `body_limit` appears correctly in `--print-config` output.

### This Step
Add `default_body_limit_is_100mb` test to `tests/e2e/cli.rs` following the pattern of `default_timeout_is_300`.

## Prerequisites
- Step 03 complete (CLI `--body-limit` flag and `print_config` output implemented)

## Files to Read Before Starting
- `tests/e2e/cli.rs:47-52` — `default_timeout_is_300` test pattern

## Implementation

### Task 1: Add `default_body_limit_is_100mb` test

In `tests/e2e/cli.rs`, after the `default_timeout_is_300` test (around line 52), add:

```rust
#[test]
fn default_body_limit_is_100mb() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("body_limit = 104857600"),
        "stdout:\n{stdout}"
    );
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo nextest run --test e2e default_body_limit_is_100mb` exits with code 0
- [ ] `cargo clippy --tests -- -D warnings` exits with code 0
- [ ] `grep -n 'default_body_limit_is_100mb' tests/e2e/cli.rs` outputs a line (test exists)

## Reviewer Instructions

You are reviewing Step 06. Verify:
1. Run `cargo nextest run --test e2e default_body_limit_is_100mb` — must exit 0
2. Run `cargo clippy --tests -- -D warnings` — must exit 0
3. Check `tests/e2e/cli.rs`:
   - `default_body_limit_is_100mb` test exists
   - Test calls `lcp().arg("--print-config")` and asserts `stdout.contains("body_limit = 104857600")`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-06: e2e-cli-test" and revert it.
