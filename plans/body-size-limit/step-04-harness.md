# Step 04: Update Test Harness with `body_limit`

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 2 adds tests. This step updates the test harness so integration tests can configure the body limit.

### This Step
Add `body_limit_bytes: u64` field and `.body_limit()` builder method to `TestHarnessBuilder` in `tests/common/harness.rs`. Wire the field into `ServerConfig` construction.

## Prerequisites
- Step 01 complete (`ServerConfig` has `body_limit_bytes: u64`)
- Step 03 complete (CLI wiring done, so we know the default value)

## Files to Read Before Starting
- `tests/common/harness.rs:85-165` — `TestHarnessBuilder` struct and `build()` method

## Implementation

### Task 1: Add `body_limit_bytes` field to `TestHarnessBuilder`

After the `timeout_seconds: u64` field (line 87), add:

```rust
body_limit_bytes: u64,
```

### Task 2: Initialize field in `TestHarnessBuilder::new()`

After `timeout_seconds: 30,` (line 97), add:

```rust
body_limit_bytes: 104_857_600,
```

This matches the server's default (100 MiB).

### Task 3: Add `.body_limit()` builder method

After the `timeout()` method (lines 109-113), add:

```rust
/// Set proxy body limit in bytes (default: 104_857_600 = 100 MiB).
/// Use 0 for no limit.
pub fn body_limit(mut self, bytes: u64) -> Self {
    self.body_limit_bytes = bytes;
    self
}
```

### Task 4: Wire into `ServerConfig` construction

In the `build()` method's `ServerConfig { ... }` block (around lines 147-157), add after `timeout_seconds`:

```rust
body_limit_bytes: self.body_limit_bytes,
```

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo nextest run` exits with code 0
- [ ] `cargo clippy --tests -- -D warnings` exits with code 0
- [ ] `grep -n 'body_limit_bytes' tests/common/harness.rs` outputs at least 3 lines (field, init, wiring)
- [ ] `grep -n 'fn body_limit' tests/common/harness.rs` outputs a line (builder method exists)

## Reviewer Instructions

You are reviewing Step 04. Verify:
1. Run `cargo nextest run` — must exit 0
2. Run `cargo clippy --tests -- -D warnings` — must exit 0
3. Check `tests/common/harness.rs`:
   - `TestHarnessBuilder` has `body_limit_bytes: u64` field
   - `new()` initializes `body_limit_bytes: 104_857_600`
   - `.body_limit(bytes: u64)` builder method exists
   - `ServerConfig` construction includes `body_limit_bytes: self.body_limit_bytes`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-04: harness" and revert it.
