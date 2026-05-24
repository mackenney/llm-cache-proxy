# Step 03: Wire JoinSet::spawn and channel capacity into handle()

## Context

### Overall Objective
Clean up test infrastructure and proxy internals: deterministic cache-write synchronization, configurable channel capacity, and accurate field naming.

### Phase Context
Wave 1 — depends on step-02 which added the `background_writes` field to `AppState` and `stream_channel_capacity` to `ServerConfig`. This step wires both into the actual request handler.

### This Step
Replace the fire-and-forget `tokio::spawn` in `handle()` with `JoinSet::spawn` via `state.background_writes`, and replace the hardcoded `STREAM_CHANNEL_CAPACITY` const with `state.config.stream_channel_capacity`. After this step, background cache writes are tracked in the JoinSet and can be awaited via `wait_for_pending_writes()`.

## Prerequisites
- Step 02 complete (AppState has `background_writes`, ServerConfig has `stream_channel_capacity`)

## Files to Read Before Starting
- `crates/lcp-server/src/proxy.rs` — `handle()` function, specifically lines 137–138 (channel const + creation) and line 152 (`tokio::spawn`)

## Implementation

### Task 1: Replace channel capacity const with config read
In `crates/lcp-server/src/proxy.rs`, replace lines 137–138:
```rust
    const STREAM_CHANNEL_CAPACITY: usize = 32;
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(STREAM_CHANNEL_CAPACITY);
```
with:
```rust
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(
        state.config.stream_channel_capacity,
    );
```

### Task 2: Replace `tokio::spawn` with `JoinSet::spawn`
In `crates/lcp-server/src/proxy.rs`, replace line 152:
```rust
    tokio::spawn(async move {
```
with:
```rust
    {
        let mut set = state.background_writes.lock().await;
        set.spawn(async move {
```

And after the closing of the spawn block (line 213, the `});`), change to:
```rust
        });
    }
```

The full pattern becomes:
```rust
    {
        let mut set = state.background_writes.lock().await;
        set.spawn(async move {
            // ... existing async block body unchanged ...
        });
    }
```

**Important:** The async block body (lines 153–212) stays exactly the same. Only the spawn mechanism changes.

**Important:** The `state` variable is moved into the handler via `State(state): State<AppState>`. After this change, `state` is used for `state.background_writes.lock()` AND `state.config.stream_channel_capacity` before the spawn. Inside the spawn block, cloned values are used (already cloned on lines 141–149). Verify that `state` is not moved into the async block — it shouldn't be, since only pre-cloned values are captured.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build` exits with code 0
- [ ] `cargo nextest run` exits with code 0 (all tests pass — yield_now still works as a rough sync, JoinSet just tracks the tasks now)
- [ ] `grep -n 'STREAM_CHANNEL_CAPACITY' crates/lcp-server/src/proxy.rs` returns no matches (const deleted)
- [ ] `grep -n 'stream_channel_capacity' crates/lcp-server/src/proxy.rs` returns a match showing config read
- [ ] `grep -n 'tokio::spawn' crates/lcp-server/src/proxy.rs` returns no matches (replaced with JoinSet::spawn)
- [ ] `grep -n 'set.spawn' crates/lcp-server/src/proxy.rs` returns a match

## Reviewer Instructions

You are reviewing Step 03 implementation. Verify:

1. Run `cargo nextest run` — must exit 0
2. Check `crates/lcp-server/src/proxy.rs` — no `const STREAM_CHANNEL_CAPACITY` exists
3. Check `crates/lcp-server/src/proxy.rs` — channel created with `state.config.stream_channel_capacity`
4. Check `crates/lcp-server/src/proxy.rs` — `tokio::spawn` replaced with `state.background_writes.lock().await` + `set.spawn(async move { ... })`
5. Check that the async block body inside spawn is unchanged (still reads upstream, forwards chunks, writes cache on success)
6. Confirm `state` is not accidentally moved into the async block (would cause compile error)
7. Run `cargo clippy` — no new warnings

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert HEAD` (single commit)
