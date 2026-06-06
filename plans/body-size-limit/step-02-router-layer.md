# Step 02: Apply `DefaultBodyLimit` to Proxy Routes

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 0 modifies lcp-server first because the CLI and test harness both depend on `ServerConfig`'s shape. This step and step-01 can run in parallel since they touch different files.

### This Step
Apply `axum::extract::DefaultBodyLimit` layer to the proxy routes (`/{provider}/{*path}`) only. Admin endpoints (`/`, `/stats`, `/cache`, `/trace`) keep Axum's default limit since they have no large request bodies.

## Prerequisites
- Step 01 must be complete (or at least: you must know that `state.config.body_limit_bytes` exists as `u64`).

## Files to Read Before Starting
- `crates/lcp-server/src/router.rs` — current `build_router` function
- `crates/lcp-server/src/server.rs:14-35` — `ServerConfig` struct with `body_limit_bytes` field

## Implementation

### Task 1: Add import for `DefaultBodyLimit`

Add this import at the top of `crates/lcp-server/src/router.rs`:

```rust
use axum::extract::DefaultBodyLimit;
```

### Task 2: Restructure `build_router` to apply limit only to proxy routes

Modify the `build_router` function to:
1. Create the proxy routes as a separate `Router`
2. Apply `DefaultBodyLimit` layer to that sub-router
3. Merge with admin routes

**When `body_limit_bytes == 0`:** use `DefaultBodyLimit::disable()` (no limit).
**When `body_limit_bytes > 0`:** use `DefaultBodyLimit::max(body_limit_bytes as usize)`.

Replace the function body with:

```rust
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.body_limit_bytes;

    // Proxy routes — apply configurable body limit
    let proxy_routes = Router::new()
        .route("/{provider}/{*path}", post(proxy::handle))
        .route("/{provider}/{*path}", get(proxy::handle));

    let proxy_routes = if body_limit == 0 {
        proxy_routes.layer(DefaultBodyLimit::disable())
    } else {
        proxy_routes.layer(DefaultBodyLimit::max(body_limit as usize))
    };

    // Admin routes — keep Axum's default 2 MiB limit
    Router::new()
        .route("/", get(stats::health))
        .route("/stats", get(stats::get_stats))
        .route("/stats", delete(stats::clear_stats))
        .route("/cache", delete(stats::clear_cache))
        .route("/cache/{key}", get(stats::get_cache_entry))
        .route("/trace/{trace_id}", get(stats::get_trace))
        .merge(proxy_routes)
        .layer(RequestDecompressionLayer::new())
        .with_state(state)
}
```

**Note:** `RequestDecompressionLayer` is applied to the merged router so it runs on all routes (both admin and proxy). Because `RequestDecompressionLayer` is the outermost middleware (Tower LIFO order), `DefaultBodyLimit` limits the **decompressed** body size. This is the desired behavior — LLM request bodies are virtually never compressed by clients, so the limit reflects the logical payload size.

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo build -p lcp-server` exits with code 0
- [ ] `cargo clippy -p lcp-server -- -D warnings` exits with code 0
- [ ] `grep -n 'DefaultBodyLimit' crates/lcp-server/src/router.rs` outputs at least 2 lines (import + usage)

## Reviewer Instructions

You are reviewing Step 02. Verify:
1. Run `cargo build -p lcp-server` — must exit 0
2. Run `cargo clippy -p lcp-server -- -D warnings` — must exit 0
3. Check `crates/lcp-server/src/router.rs`:
   - Import for `axum::extract::DefaultBodyLimit` exists
   - `body_limit == 0` case uses `DefaultBodyLimit::disable()`
   - `body_limit > 0` case uses `DefaultBodyLimit::max(body_limit as usize)`
   - Limit is applied only to proxy routes, not admin routes

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-02: router-layer" and revert it.
