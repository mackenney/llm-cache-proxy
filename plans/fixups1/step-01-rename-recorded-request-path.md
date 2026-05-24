# Step 01: Rename RecordedRequest.path → uri

## Context

### Overall Objective
Clean up test infrastructure and proxy internals: deterministic cache-write synchronization, configurable channel capacity, and accurate field naming.

### Phase Context
Wave 0 — this step is fully independent of all other steps. It touches only test infrastructure files and has no interaction with the proxy server code.

### This Step
Rename `RecordedRequest.path` (which stores a full URI including query string) to `uri` for accuracy. Add a `.path()` method that strips the query string, giving callers a clean API. Two call sites need updating; the compiler enforces completeness.

## Prerequisites
- None

## Files to Read Before Starting
- `tests/common/mock_upstream.rs` — struct definition (line 18–23), construction (line 178–183), self-test assertion (line 237)
- `tests/common/harness.rs` — caller at line 199

## Implementation

### Task 1: Rename field in struct definition
In `tests/common/mock_upstream.rs` line 20, change:
```rust
pub path: String,
```
to:
```rust
pub uri: String,
```

### Task 2: Add `.path()` method
Add an `impl RecordedRequest` block after the struct definition (after line 23):
```rust
impl RecordedRequest {
    pub fn path(&self) -> &str {
        self.uri.split_once('?').map_or(self.uri.as_str(), |(p, _)| p)
    }
}
```

### Task 3: Update construction site
In `tests/common/mock_upstream.rs` line 180, change:
```rust
path: uri.to_string(),
```
to:
```rust
uri: uri.to_string(),
```

### Task 4: Update mock self-test assertion
In `tests/common/mock_upstream.rs` line 237, change:
```rust
assert_eq!(reqs[0].path, "/test");
```
to:
```rust
assert_eq!(reqs[0].path(), "/test");
```
This uses the new method, which is correct — the test expects a path without query string.

### Task 5: Update harness test assertion
In `tests/common/harness.rs` line 199, change:
```rust
assert!(reqs[0].path.contains("v1/messages"));
```
to:
```rust
assert!(reqs[0].path().contains("v1/messages"));
```
The intent is endpoint verification — `.path()` is semantically correct here.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build --tests` exits with code 0 (no compile errors from rename)
- [ ] `cargo nextest run` exits with code 0 (no regressions)
- [ ] `grep -rn '\.path' tests/common/mock_upstream.rs` shows only the new method definition and its usage — no remaining `reqs[N].path` field access (only `reqs[N].path()` method calls)
- [ ] `grep -n 'pub path:' tests/common/mock_upstream.rs` returns no matches (field is gone)
- [ ] `grep -n 'pub uri:' tests/common/mock_upstream.rs` returns exactly one match (new field)

## Reviewer Instructions

You are reviewing Step 01 implementation. Verify:

1. Run `cargo nextest run` — must exit 0
2. Check `tests/common/mock_upstream.rs` — `RecordedRequest` has field `pub uri: String` (not `path`)
3. Check `tests/common/mock_upstream.rs` — `impl RecordedRequest` block with `pub fn path(&self) -> &str` exists
4. Check `tests/common/mock_upstream.rs` line ~180 — construction uses `uri: uri.to_string()`
5. Check `tests/common/mock_upstream.rs` line ~237 — assertion uses `reqs[0].path()` (method call)
6. Check `tests/common/harness.rs` line ~199 — assertion uses `reqs[0].path()` (method call)
7. Run `grep -rn '\.path[^(]' tests/common/` — no remaining direct field access to `.path` on RecordedRequest

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert HEAD` (single commit)
