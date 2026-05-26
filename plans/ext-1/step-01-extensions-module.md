# Step 01: Extensions Module

## Context

### Overall Objective
Add a three-phase extension pipeline to `lcp-server`: Phase 1 (pre-cache-key
body transform), Phase 2 (pre-wire body transform + sensitive state production),
Phase 3 (response stream transform consuming sensitive state).

### Phase Context
Wave 0. No existing code to integrate yet — define all types and the trait
in isolation so Wave 1 can wire them in.

### This Step
Create `crates/lcp-server/src/extensions.rs` with:
- `SensitiveStateBuilder` and `SensitiveState` types
- `ProxyCtx` value type
- `Extension` trait
- `ExtensionPipeline` runner struct

No changes to any existing file except exporting the new module from `lib.rs`.

## Prerequisites
None.

## Files to Read Before Starting
- `crates/lcp-server/SPEC.md` — Extension Pipeline section (at the bottom)
  defines every invariant this step must satisfy
- `crates/lcp-server/src/lib.rs` — to know what's already exported
- `crates/lcp-server/src/proxy.rs` — understand `Provider`, `Bytes`, stream
  types already in use

## Implementation

### Task 1: SensitiveStateBuilder and SensitiveState

```rust
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Construction surface for SensitiveState. Produced and filled in Phase 2,
/// sealed via `build()` before being handed to the framework.
#[derive(Default)]
pub struct SensitiveStateBuilder {
    inner: HashMap<String, String>,
}

impl SensitiveStateBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.inner.insert(key.into(), value.into());
        self
    }

    pub fn build(self) -> SensitiveState {
        SensitiveState {
            inner: Arc::new(self.inner),
        }
    }
}

/// Immutable per-request, per-extension state store.
///
/// Carries extension state across the Phase 2 → Phase 3 boundary.
/// The framework guarantees:
/// - Contents are never logged, traced, or printed (Debug renders as `<redacted>`).
/// - Never shared across extensions.
/// - Never persisted to any storage medium.
/// - Dropped when the request's response stream is fully consumed.
#[derive(Clone)]
pub struct SensitiveState {
    inner: Arc<HashMap<String, String>>,
}

impl SensitiveState {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key).map(|s| s.as_str())
    }
}

/// Debug MUST NOT reveal contents — this is part of the non-inspection guarantee.
impl fmt::Debug for SensitiveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SensitiveState")
            .field("contents", &"<redacted>")
            .finish()
    }
}
```

### Task 2: ProxyCtx

```rust
use lcp_core::Provider;

/// Read-only snapshot of the in-flight request passed to extension hooks.
/// `cache_key` is absent in Phase 1 (not yet computed) and present in
/// Phase 2 and Phase 3.
#[derive(Clone, Debug)]
pub struct ProxyCtx {
    pub provider: Provider,
    pub method: String,
    pub path: String,
    pub cache_key: Option<String>,
}
```

### Task 3: Extension trait

Use `futures_util::future::BoxFuture` for object-safe async hooks. All three
hook methods have default (identity) implementations so an extension that only
needs one hook doesn't need to implement the others.

```rust
use bytes::Bytes;
use futures_util::future::BoxFuture;
use std::pin::Pin;
use futures_util::Stream;

pub type ResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

pub trait Extension: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// Phase 1 — fires before cache key computation on every proxied request.
    /// Default: return `body` unchanged.
    fn on_request_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<Bytes, anyhow::Error>> {
        Box::pin(async move { Ok(body) })
    }

    /// Phase 2 — fires on cache miss, after cache key computation, before
    /// the upstream request is sent. Returns the body to send over the wire
    /// and a SensitiveStateBuilder that the framework seals and holds for
    /// Phase 3. Default: return `body` unchanged and an empty state.
    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move { Ok((body, SensitiveStateBuilder::new())) })
    }

    /// Phase 3 — fires on cache miss, after the upstream responds. Wraps the
    /// response stream. The `state` is the SensitiveState sealed from the
    /// SensitiveStateBuilder produced by this same extension's Phase 2 hook
    /// for this same request. Default: return `stream` unchanged.
    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        _state: SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        stream
    }
}
```

### Task 4: ExtensionPipeline

`ExtensionPipeline` stores boxed extensions and provides `run_phase1`,
`run_phase2`, and `run_phase3`. It derives `Clone` (all extensions are
`Arc`-wrapped internally) and `Default` (empty pipeline = no-op).

```rust
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct ExtensionPipeline {
    extensions: Vec<Arc<dyn Extension>>,
}

impl ExtensionPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extension. Extensions are called in registration order.
    pub fn add(mut self, ext: impl Extension) -> Self {
        self.extensions.push(Arc::new(ext));
        self
    }

    /// Phase 1: run all on_request_body hooks in order before cache key
    /// computation. Errors fail closed.
    pub async fn run_phase1(
        &self,
        ctx: ProxyCtx,
        mut body: Bytes,
    ) -> Result<Bytes, anyhow::Error> {
        for ext in &self.extensions {
            body = ext.on_request_body(ctx.clone(), body).await?;
        }
        Ok(body)
    }

    /// Phase 2: run all on_upstream_body hooks in order after cache key
    /// computation. Returns the wire body and one SensitiveState per
    /// extension (indexed by registration order). Errors fail closed.
    pub async fn run_phase2(
        &self,
        ctx: ProxyCtx,
        mut body: Bytes,
    ) -> Result<(Bytes, Vec<SensitiveState>), anyhow::Error> {
        let mut states = Vec::with_capacity(self.extensions.len());
        for ext in &self.extensions {
            let (new_body, builder) = ext.on_upstream_body(ctx.clone(), body).await?;
            body = new_body;
            states.push(builder.build());
        }
        Ok((body, states))
    }

    /// Phase 3: wrap the response stream through all on_response_stream hooks
    /// in order. Each extension receives only its own SensitiveState.
    /// `states` must be the Vec returned by run_phase2 for this request.
    pub fn run_phase3(
        &self,
        ctx: ProxyCtx,
        states: Vec<SensitiveState>,
        mut stream: ResponseStream,
    ) -> ResponseStream {
        for (ext, state) in self.extensions.iter().zip(states.into_iter()) {
            stream = ext.on_response_stream(ctx.clone(), state, stream);
        }
        stream
    }
}
```

### Task 5: Export from lib.rs

Add to `crates/lcp-server/src/lib.rs`:

```rust
pub mod extensions;
```

Also re-export the public surface so callers don't need to spell out the module:

```rust
pub use extensions::{Extension, ExtensionPipeline, ProxyCtx, SensitiveState, SensitiveStateBuilder, ResponseStream};
```

## Acceptance Criteria

- [ ] `cargo build -p lcp-server` exits 0
- [ ] `cargo clippy -p lcp-server -- -D warnings` exits 0
- [ ] `cargo test -p lcp-server` exits 0 (no regressions)
- [ ] `grep -n 'contents.*<redacted>' crates/lcp-server/src/extensions.rs` finds the Debug impl
- [ ] `grep -n 'pub fn get' crates/lcp-server/src/extensions.rs` finds exactly one method on SensitiveState
- [ ] `grep -n 'iter\|serialize\|Display' crates/lcp-server/src/extensions.rs` finds zero results in the SensitiveState impl block

## Reviewer Instructions

You are reviewing Step 01 implementation. Verify:

1. Run `cargo build -p lcp-server` — must exit 0
2. Run `cargo clippy -p lcp-server -- -D warnings` — must exit 0
3. Run `cargo test -p lcp-server` — must exit 0, no regressions
4. Check `crates/lcp-server/src/extensions.rs`:
   - `SensitiveState::fmt` renders `<redacted>`, not actual contents
   - `SensitiveState` has no `iter`, `serialize`, `Display`, or `IntoIterator` impl
   - `ExtensionPipeline::run_phase2` returns `Vec<SensitiveState>` indexed by extension order
   - `ExtensionPipeline::run_phase3` calls `ext.on_response_stream` with `states[i]` for extension `i` only
5. Check `crates/lcp-server/src/lib.rs` exports the new module

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit for this step. No other files are touched.
