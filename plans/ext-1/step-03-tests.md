# Step 03: Tests

## Context

### Overall Objective
Add spec invariant tests for the extension pipeline covering isolation,
ordering, phase firing rules, fail-closed behavior, and SensitiveState
non-inspection guarantees.

### Phase Context
Wave 2. Both `extensions.rs` and the proxy integration are complete.
Tests exercise the pipeline end-to-end using the existing `TestHarness`
/ `MockUpstream` infrastructure.

### This Step
Write tests that cover the behavioral invariants from the Extension Pipeline
section of `crates/lcp-server/SPEC.md`. Tests live in `tests/` (external
tier) so they are behavioral contracts, not implementation details.

## Prerequisites
- Step 01 and Step 02 complete and committed.

## Files to Read Before Starting
- `crates/lcp-server/SPEC.md` — Extension Pipeline section
- `tests/src/` — existing test helpers (TestHarness, MockUpstream) to
  understand the test patterns already in use
- `crates/lcp-server/src/extensions.rs` — types under test

## Implementation

Write a new test file `tests/src/spec/extensions.rs` (or integrate into
the existing spec module following the pattern already there). Cover the
following invariants — each as a separate `#[test]` or `#[tokio::test]`:

### INV-EXT-1: Empty pipeline is transparent
A proxy with no extensions registered returns responses identical to a
direct passthrough. Body, status, and cache headers are unchanged.

### INV-EXT-2: Phase 1 transforms the cache key input
Register an extension whose `on_request_body` appends a byte to the body.
Send the same logical request twice. Confirm the two requests hash to the
same key (because the extension is deterministic) — i.e., a cache hit
occurs on the second call.

A simpler variant: send two requests where the raw bodies differ but the
extension normalizes them to the same bytes. Confirm the second call is a
cache hit.

### INV-EXT-3: Phase 2 fires on miss, not on hit
Register an extension that records whether `on_upstream_body` was called
(e.g., writes to an `Arc<AtomicBool>`). Trigger a cache miss, then a cache
hit for the same request. Assert the flag is true after the miss and still
true (not incremented a second time) after the hit.

### INV-EXT-4: Phase 3 receives the SensitiveState produced by Phase 2
Register an extension that:
- In `on_upstream_body`: writes `"ping"` → `"pong"` into the builder.
- In `on_response_stream`: reads `state.get("ping")` and panics if it is
  not `Some("pong")`.
A successful proxy round-trip (no panic) confirms state flows correctly.

### INV-EXT-5: SensitiveState is per-extension
Register two extensions, A and B. Extension A writes `"key_a"` in Phase 2.
Extension B writes `"key_b"` in Phase 2. In Phase 3:
- Extension A asserts `state.get("key_b")` is `None`.
- Extension B asserts `state.get("key_a")` is `None`.

### INV-EXT-6: Phase 1 fires on bypass requests
Register an extension that increments an `Arc<AtomicUsize>` in
`on_request_body`. Issue a request with `x-lcp-bypass: 1`. Assert the
counter is 1. Then confirm `on_upstream_body` was NOT called (separate
counter = 0).

### INV-EXT-7: Phase 2 error fails closed, upstream not reached
Register an extension whose `on_upstream_body` returns an `Err(...)`.
Issue a proxied request. Assert:
- The client receives a 5xx response.
- The `MockUpstream` received zero requests (upstream was not contacted).

### INV-EXT-8: Phase 3 wraps the stream that is cached
Register an extension that replaces every byte `b'X'` with `b'Y'` in the
response stream (a simple stream map). Trigger a cache miss. Then retrieve
the cached exchange via `GET /cache/<key>`. Assert the cached body contains
`b'Y'` bytes, not `b'X'`.

### INV-EXT-9: Debug output of SensitiveState does not reveal contents

Unit test (not integration — lives in `extensions.rs` under `#[cfg(test)]`):

```rust
#[test]
fn sensitive_state_debug_is_redacted() {
    let mut b = SensitiveStateBuilder::new();
    b.set("secret_key", "super_secret_value");
    let state = b.build();
    let debug_output = format!("{:?}", state);
    assert!(!debug_output.contains("super_secret_value"));
    assert!(!debug_output.contains("secret_key"));
    assert!(debug_output.contains("redacted"));
}
```

### INV-EXT-10: Extensions run in registration order
Register two extensions that each append their name to a shared
`Arc<Mutex<Vec<String>>>` in `on_request_body`. Assert the names appear
in registration order after a request.

## Acceptance Criteria

- [ ] `cargo nextest run` exits 0
- [ ] `cargo nextest run --test spec` exits 0 and output mentions at least
  7 of the INV-EXT-* test names
- [ ] `grep -rn 'INV-EXT' tests/` finds at least 9 distinct invariant labels
- [ ] INV-EXT-9 is present as a unit test in `crates/lcp-server/src/extensions.rs`
- [ ] No existing test is modified or deleted

## Reviewer Instructions

You are reviewing Step 03 implementation. Verify:

1. Run `cargo nextest run` — must exit 0
2. Run `cargo nextest run --test spec` — must exit 0, confirm INV-EXT-* tests appear in output
3. Check `tests/` for new extension test file — confirm at least INV-EXT-1 through INV-EXT-8 are present
4. Check `crates/lcp-server/src/extensions.rs` — confirm INV-EXT-9 unit test is present
5. Confirm no existing test was weakened or removed

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit for this step.
