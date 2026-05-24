# Step 04: Integration Validation and Cleanup

## Objective
Validate that the streaming implementation is complete and correct. Run all tests, check for regressions, verify clippy compliance, and confirm the implementation matches the spec.

## Depends On
- step-03 (all streaming code must be in place)

## Validation Tasks

### 1. Full Test Suite

```bash
cargo nextest run 2>&1 | tail -30
```
Expected: all tests pass

### 2. Clippy with No Dead Code Allowance

```bash
cargo clippy --all-targets 2>&1 | grep -E "^(error|warning:.*unused)" || echo "Clean"
```
Expected: "Clean" (no unused code warnings after full integration)

### 3. Format Check

```bash
cargo fmt --check 2>&1 || echo "Format issues found"
```
Expected: no output (already formatted)

### 4. Build Release Mode

```bash
cargo build --release 2>&1 | tail -10
```
Expected: compiles successfully

## Code Review Checklist

Verify these invariants are preserved in the final code:

### In `handle()` function:
- [ ] `ReceiverStream::new(rx)` wraps the channel receiver
- [ ] `Body::from_stream(body_stream)` creates the response body
- [ ] `tokio::spawn` creates the background task
- [ ] `mpsc::channel::<Result<Bytes, std::io::Error>>(32)` uses bounded channel
- [ ] `stream_complete` flag tracks successful completion
- [ ] Cache write only happens when `stream_complete && do_cache`
- [ ] `drop(tx)` signals end-of-stream
- [ ] Client disconnect detected via `tx.send().is_err()`
- [ ] SSE headers (`cache-control`, `transfer-encoding`) still applied for SSE responses

### In `serve_cached()` function:
- [ ] Uses `futures_util::stream::iter()` to convert chunks
- [ ] Uses `Body::from_stream()` to create body
- [ ] Each chunk yields separately (no flattening/concatenation)
- [ ] Headers include `x-lcp-cache: HIT`

### In imports:
- [ ] `use std::pin::Pin;`
- [ ] `use std::task::{Context, Poll};`
- [ ] `use tokio::sync::mpsc;`
- [ ] `use futures_util::{Stream, StreamExt};`

## Optional: Manual E2E Test

If the test infrastructure supports it:

```bash
# Terminal 1: Start the proxy
cargo run -- --port 8080 --cache-dir /tmp/lcp-test

# Terminal 2: Make a streaming request (requires API key and endpoint)
curl -N http://localhost:8080/openai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Count to 5"}],"stream":true}' \
  | head -20

# Verify: chunks arrive incrementally, not all at once
# Verify: second identical request returns instantly (cache hit)
```

## Acceptance Criteria

1. **All tests pass:**
   ```bash
   cargo nextest run 2>&1 | grep -E "(PASS|FAIL|passed|failed)" | tail -5
   ```
   Expected: "X tests passed" with no failures

2. **No clippy errors:**
   ```bash
   cargo clippy --all-targets 2>&1 | grep "^error" || echo "No errors"
   ```
   Expected: "No errors"

3. **Code formatted:**
   ```bash
   cargo fmt --check
   ```
   Expected: exit code 0

4. **Release build succeeds:**
   ```bash
   cargo build --release 2>&1 | grep -E "^error" || echo "Build OK"
   ```
   Expected: "Build OK"

## Post-Completion

After all acceptance criteria pass:
1. Mark this step complete in PROGRESS.md
2. Update plan status to "Complete"
3. The plan can be deleted per project conventions

## Commit Message
```
step-04: validate streaming implementation
```

Note: This step may not require a commit if no code changes are needed. If formatting or minor fixes are required, include them in this commit.
