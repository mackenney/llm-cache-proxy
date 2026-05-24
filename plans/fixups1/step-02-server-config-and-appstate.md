# Step 02: Add stream_channel_capacity to ServerConfig, JoinSet to AppState, refactor build_router

## Context

### Overall Objective
Clean up test infrastructure and proxy internals: deterministic cache-write synchronization, configurable channel capacity, and accurate field naming.

### Phase Context
Wave 0 — this step lays the structural foundation (new fields, new method, new signature) that step-03 wires into the hot path. It is parallelizable with step-01 (no file overlap).

### This Step
Three additive changes that compile but are not yet wired into behavior:
1. Add `stream_channel_capacity: usize` to `ServerConfig` so channel size is configurable.
2. Add `background_writes: Arc<tokio::sync::Mutex<JoinSet<()>>>` to `AppState` and a `wait_for_pending_writes()` method.
3. Change `build_router` to accept `AppState` directly (instead of separate `Arc<ServerConfig>` + `Arc<Client>`), so callers can retain a reference to `AppState`.

After this step, existing code still compiles — `serve()` and `TestHarnessBuilder::build()` are updated to construct `AppState` and pass it to `build_router`. The `tokio::spawn` in `handle()` is NOT changed yet (that's step-03).

## Prerequisites
- None

## Files to Read Before Starting
- `crates/lcp-server/src/server.rs` — `ServerConfig` struct (lines 12–22), `serve()` function (lines 48–68)
- `crates/lcp-server/src/proxy.rs` — `AppState` struct (lines 42–46)
- `crates/lcp-server/src/router.rs` — `build_router` function (lines 11–25)
- `tests/common/harness.rs` — `TestHarnessBuilder::build()` (lines 106–148)

## Implementation

### Task 1: Add `stream_channel_capacity` to `ServerConfig`
In `crates/lcp-server/src/server.rs`, add after line 21 (`pub gemini_upstream: Option<String>`):
```rust
    /// Bounded channel capacity for streaming response chunks. Default: 32.
    pub stream_channel_capacity: usize,
```

### Task 2: Add `JoinSet` and `wait_for_pending_writes` to `AppState`
In `crates/lcp-server/src/proxy.rs`:

Add to imports (top of file):
```rust
use tokio::sync::Mutex;
use tokio::task::JoinSet;
```

Update the `AppState` struct (lines 42–46) to:
```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub client: Arc<reqwest::Client>,
    pub background_writes: Arc<Mutex<JoinSet<()>>>,
}
```

Add after the struct:
```rust
impl AppState {
    pub async fn wait_for_pending_writes(&self) {
        let mut set = self.background_writes.lock().await;
        while set.join_next().await.is_some() {}
    }
}
```

Design rationale: `tokio::sync::Mutex` (not `std::sync::Mutex`) because `JoinSet::join_next()` is async and must be called inside the lock. The hot-path lock (`spawn` in step-03) holds it only for the `spawn()` call duration — sub-microsecond, no contention concern.

### Task 3: Change `build_router` signature
In `crates/lcp-server/src/router.rs`, change from:
```rust
pub fn build_router(config: Arc<ServerConfig>, client: Arc<reqwest::Client>) -> Router {
    let state = AppState { config, client };
```
to:
```rust
pub fn build_router(state: AppState) -> Router {
```
Remove the `use crate::server::ServerConfig;` import if it becomes unused. Keep `use crate::proxy::AppState;`.

### Task 4: Update `serve()` to construct `AppState`
In `crates/lcp-server/src/server.rs`, update the `serve()` function. After the `client` construction (line 61), add `AppState` construction and pass it to `build_router`:

```rust
    let state = crate::proxy::AppState {
        config,
        client,
        background_writes: Arc::new(tokio::sync::Mutex::new(tokio::task::JoinSet::new())),
    };

    let app = build_router(state);
```

Replace the existing `let app = build_router(config, client);` line (63).

### Task 5: Update `TestHarnessBuilder::build()` to construct `AppState` and store it
In `tests/common/harness.rs`:

Add import at top:
```rust
use lcp_server::proxy::AppState;
```

Add field to `TestHarness`:
```rust
pub struct TestHarness {
    pub mock: MockUpstream,
    cache: Cache,
    proxy_addr: SocketAddr,
    proxy_handle: Option<JoinHandle<()>>,
    app_state: AppState,
}
```

In `TestHarnessBuilder::build()` (around lines 112–130), replace the config/client/router construction:
```rust
        let config = ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            cache: cache.clone(),
            timeout_seconds: self.timeout_seconds,
            anthropic_upstream: Some(mock_url.clone()),
            openai_upstream: Some(mock_url.clone()),
            openrouter_upstream: Some(mock_url.clone()),
            gemini_upstream: Some(mock_url.clone()),
            stream_channel_capacity: 32,
        };

        let client = Arc::new(
            reqwest::Client::builder()
                .no_gzip()
                .no_deflate()
                .no_brotli()
                .build()
                .expect("build reqwest client"),
        );

        let app_state = AppState {
            config: Arc::new(config.clone()),
            client,
            background_writes: Arc::new(tokio::sync::Mutex::new(tokio::task::JoinSet::new())),
        };

        let app = build_router(app_state.clone());
```

Update the `TestHarness` construction to include `app_state`:
```rust
        TestHarness {
            mock,
            cache,
            proxy_addr,
            proxy_handle: Some(proxy_handle),
            app_state,
        }
```

Add the `wait_for_writes` method to `TestHarness`:
```rust
    pub async fn wait_for_writes(&self) {
        self.app_state.wait_for_pending_writes().await;
    }
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build` exits with code 0 (library compiles)
- [ ] `cargo build --tests` exits with code 0 (tests compile)
- [ ] `cargo nextest run` exits with code 0 (all 41+ tests pass — JoinSet is constructed but not yet used by `handle()`)
- [ ] `grep -n 'stream_channel_capacity' crates/lcp-server/src/server.rs` shows the field definition
- [ ] `grep -n 'background_writes' crates/lcp-server/src/proxy.rs` shows the field and usage in `wait_for_pending_writes`
- [ ] `grep -n 'pub fn build_router' crates/lcp-server/src/router.rs` shows signature accepts `AppState` (not two separate args)
- [ ] `grep -n 'wait_for_writes' tests/common/harness.rs` shows the method exists on `TestHarness`

## Reviewer Instructions

You are reviewing Step 02 implementation. Verify:

1. Run `cargo nextest run` — must exit 0
2. Check `crates/lcp-server/src/server.rs` — `ServerConfig` has `pub stream_channel_capacity: usize`
3. Check `crates/lcp-server/src/proxy.rs` — `AppState` has `pub background_writes: Arc<Mutex<JoinSet<()>>>`
4. Check `crates/lcp-server/src/proxy.rs` — `impl AppState` has `pub async fn wait_for_pending_writes(&self)`
5. Check `crates/lcp-server/src/router.rs` — `build_router` takes `AppState` (single arg)
6. Check `crates/lcp-server/src/server.rs` — `serve()` constructs `AppState` with `background_writes` field
7. Check `tests/common/harness.rs` — `TestHarness` has `app_state: AppState` field and `wait_for_writes()` method
8. Check `tests/common/harness.rs` — `build()` passes `stream_channel_capacity: 32` in `ServerConfig`
9. Confirm `handle()` in `proxy.rs` still uses `tokio::spawn` (not yet changed — that's step-03)

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert HEAD` (single commit)
