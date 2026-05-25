# Step 02: Add .ttl() to TestHarnessBuilder

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 0: infrastructure extension. This step adds a `.ttl(seconds)` builder method to `TestHarnessBuilder` so that TTL integration tests (Wave 2) can configure cache expiry. All Wave 0 steps are independent and run in parallel.

### This Step
The current `TestHarnessBuilder::build()` hardcodes `Cache::open(&":memory:".into(), 0)` (TTL=0 = never expire). Adding a `ttl_seconds` field and `.ttl()` method lets tests customize the cache TTL, which is required to test that expired entries return MISS instead of HIT.

## Prerequisites
- None (Wave 0 step)

## Files to Read Before Starting
- `tests/common/harness.rs` — full file; understand `TestHarnessBuilder` struct fields and `build()` method
- `crates/lcp-core/src/cache.rs` (or wherever `Cache::open` is defined) — confirm the second parameter is TTL seconds

## Implementation

### Task 1: Add ttl_seconds field to TestHarnessBuilder

In `tests/common/harness.rs`, extend the `TestHarnessBuilder` struct:

```rust
pub struct TestHarnessBuilder {
    mock: Option<MockUpstream>,
    timeout_seconds: u64,
    ttl_seconds: u64,  // 0 = never expire (default)
}
```

### Task 2: Initialize ttl_seconds in new()

In `TestHarnessBuilder::new()`, add the field:

```rust
fn new() -> Self {
    Self {
        mock: None,
        timeout_seconds: 30,
        ttl_seconds: 0,
    }
}
```

### Task 3: Add .ttl() builder method

After the existing `.timeout()` method, add:

```rust
/// Set cache TTL in seconds (default: 0 = never expire).
/// Use in integration tests that verify entry expiry.
pub fn ttl(mut self, seconds: u64) -> Self {
    self.ttl_seconds = seconds;
    self
}
```

### Task 4: Use ttl_seconds in build()

In `TestHarnessBuilder::build()`, replace the hardcoded `0` in `Cache::open`:

```rust
// Before:
let cache = Cache::open(&":memory:".into(), 0).expect("open in-memory cache");

// After:
let cache = Cache::open(&":memory:".into(), self.ttl_seconds).expect("open in-memory cache");
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build --test integration` exits 0
- [ ] `cargo build --test spec` exits 0 (no compile regressions)
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)
- [ ] `grep -n 'ttl' tests/common/harness.rs` shows the field, the method, and its use in `build()`

## Reviewer Instructions

You are reviewing Step 02 implementation. Verify:

1. Run `cargo build --test integration` — must exit 0
2. Run `cargo nextest run --test spec` — must exit 0 (no regressions)
3. Run `grep -n 'ttl' tests/common/harness.rs` — must show field declaration, `new()` init, `.ttl()` method, and use in `build()`
4. Confirm `Cache::open` call in `build()` now uses `self.ttl_seconds` instead of literal `0`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert` the commit for step-02. The `ttl_seconds` field, `.ttl()` method, and updated `Cache::open` call are all additive — removal restores the original `0`-hardcoded behavior.
