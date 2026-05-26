# Step 02: Proxy Integration

## Context

### Overall Objective
Add a three-phase extension pipeline to `lcp-server` so callers can register
body transforms and response stream transforms at startup.

### Phase Context
Wave 1. `extensions.rs` from step-01 exists and compiles. This step wires
the pipeline into `ServerConfig`, `AppState`, and the `proxy::handle` function.

### This Step
- Add `ExtensionPipeline` to `ServerConfig` and propagate it into `AppState`.
- In `proxy::handle`, call `pipeline.run_phase1` before cache key computation.
- In `proxy::handle`, call `pipeline.run_phase2` on the miss path before the
  upstream request is sent.
- In the spawned background task, wrap the upstream stream with
  `pipeline.run_phase3` before chunk accumulation begins.
- Fail closed on Phase 1 and Phase 2 errors (return 5xx, do not forward).

## Prerequisites
- Step 01 complete: `crates/lcp-server/src/extensions.rs` compiles and is
  exported from `lib.rs`.

## Files to Read Before Starting
- `crates/lcp-server/src/extensions.rs` — types and pipeline API from step-01
- `crates/lcp-server/src/server.rs` — `ServerConfig` struct, `serve()` function
- `crates/lcp-server/src/proxy.rs` — full `handle` function and spawned task
- `crates/lcp-server/SPEC.md` — Extension Pipeline section for invariant details

## Implementation

### Task 1: Add pipeline to ServerConfig (server.rs)

Add `extensions: ExtensionPipeline` as the last field of `ServerConfig`.
It has a `Default` impl (empty pipeline = no-op), so existing construction
sites that use struct literal syntax must add the field; callers using
`ServerConfig { .. }` in tests will need `extensions: ExtensionPipeline::new()`.

```rust
use crate::extensions::ExtensionPipeline;

pub struct ServerConfig {
    // ... existing fields unchanged ...
    pub extensions: ExtensionPipeline,
}
```

`ServerConfig` already derives `Clone`; `ExtensionPipeline` is `Clone`, so
no change needed there.

### Task 2: Propagate into AppState (proxy.rs)

`AppState` is constructed in `server.rs::serve()`. The `config` field is
`Arc<ServerConfig>`, so the pipeline is accessible as `state.config.extensions`.
No changes to `AppState` struct are needed.

### Task 3: Phase 1 in proxy::handle (proxy.rs)

After extracting `provider`, `path`, `body` from the request — and before
calling `cache_key(...)` — run Phase 1:

```rust
let ctx = ProxyCtx {
    provider,
    method: "POST".to_owned(),   // or from the actual request method
    path: path.clone(),
    cache_key: None,
};

let body = match state.config.extensions.run_phase1(ctx.clone(), body).await {
    Ok(b) => b,
    Err(e) => {
        tracing::error!(err = %e, "extension phase 1 error");
        return (StatusCode::INTERNAL_SERVER_ERROR, "extension error").into_response();
    }
};

// cache_key uses the Phase-1-transformed body
let key = cache_key(provider, "POST", &full_path, &body);
```

Update `ctx` with the cache key before Phase 2:
```rust
let ctx = ProxyCtx { cache_key: Some(key.clone()), ..ctx };
```

### Task 4: Phase 2 on cache miss (proxy.rs)

On the miss path, after the cache lookup falls through and before constructing
the upstream request, run Phase 2:

```rust
let (wire_body, ext_states) =
    match state.config.extensions.run_phase2(ctx.clone(), body.clone()).await {
        Ok(x) => x,
        Err(e) => {
            tracing::error!(err = %e, "extension phase 2 error");
            return (StatusCode::BAD_GATEWAY, "extension error").into_response();
        }
    };
```

Use `wire_body` (not the original `body`) when constructing the upstream
`reqwest` request:
```rust
let mut rb = state.client.post(&url).body(wire_body);
```

The original `body` (pre-Phase-2) is still used for the cache write's
`RequestRecord::body` field, since the cache stores the pre-wire request.

### Task 5: Phase 3 wrapping the upstream stream (proxy.rs)

Currently the spawned background task does:
```rust
let mut stream = upstream_resp.bytes_stream();
```

Refactor to extract the stream before spawning, apply Phase 3, then move the
wrapped stream into the task:

```rust
let raw_stream: ResponseStream =
    Box::pin(upstream_resp.bytes_stream().map(|r| {
        r.map_err(|e| std::io::Error::other(e.to_string()))
    }));

let response_stream = state.config.extensions.run_phase3(
    ctx.clone(),
    ext_states,
    raw_stream,
);
```

Inside the spawned task, iterate over `response_stream` instead of
`upstream_resp.bytes_stream()`:

```rust
set.spawn(async move {
    let mut stream = response_stream;
    // rest of the accumulation/forward loop unchanged
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => { /* forward to tx, accumulate */ }
            Err(e) => { /* propagate error */ }
        }
    }
    // cache write unchanged
});
```

`response_stream` is `Pin<Box<dyn Stream + Send>>` so it moves into the
`async move` block cleanly.

For the bypass path: Phase 2 and Phase 3 MUST NOT fire. The bypass check
happens before Phase 2, so no special handling needed — just ensure the
bypass branch returns early before reaching Phase 2.

For the cache hit path: Phase 2 and Phase 3 MUST NOT fire. The hit branch
already returns early via `return serve_cached(...)`, so no changes needed.

### Task 6: Update ServerConfig construction in tests

Search for all `ServerConfig {` literals in `tests/` and add
`extensions: ExtensionPipeline::new()`. These tests exercise the proxy with
an empty (no-op) pipeline, which is the correct default.

## Acceptance Criteria

- [ ] `cargo build -p lcp-server` exits 0
- [ ] `cargo build -p lcp` exits 0
- [ ] `cargo clippy -- -D warnings` exits 0 (workspace-wide)
- [ ] `cargo nextest run` exits 0 — all existing tests pass
- [ ] `grep -n 'run_phase1' crates/lcp-server/src/proxy.rs` finds a call before the `cache_key(` call
- [ ] `grep -n 'run_phase2' crates/lcp-server/src/proxy.rs` finds a call after cache miss detection and before `state.client.post`
- [ ] `grep -n 'run_phase3' crates/lcp-server/src/proxy.rs` finds a call that produces the stream used by the spawned task
- [ ] Bypassed requests do not reach Phase 2 or Phase 3 code paths (trace the `bypass == true` branch in proxy.rs and confirm it returns before `run_phase2`)

## Reviewer Instructions

You are reviewing Step 02 implementation. Verify:

1. Run `cargo build` (workspace) — must exit 0
2. Run `cargo clippy -- -D warnings` — must exit 0
3. Run `cargo nextest run` — must exit 0, all tests pass
4. In `proxy.rs`: confirm `run_phase1` is called before `cache_key(`
5. In `proxy.rs`: confirm `run_phase2` is called only on the miss path (after the `Ok(Some(...)) => return serve_cached` arm), and before `state.client.post(`
6. In `proxy.rs`: confirm the spawned task uses the Phase-3-wrapped stream, not `upstream_resp.bytes_stream()` directly
7. Confirm the bypass branch (`if bypass { ... }` or the bypass early return) does not reach Phase 2 or Phase 3
8. Confirm the cache hit branch returns before Phase 2

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit for this step.
