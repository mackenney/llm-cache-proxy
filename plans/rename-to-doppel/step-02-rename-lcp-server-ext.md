# Step 02: Rename lcp-server Ext Layer

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` and update all consumers in lcp.

### Phase Context
Wave 1 — runs alone after step-01. Step-03 and step-04 depend on exports from
this step (`DoppelExt`), so it must complete before they can compile.

### This Step
Rename the lcp-server extension files and types to match the new `doppel` API.
`ScrubExt` → `DoppelExt`, `SseUnscrubStream` → `SseRestoreStream`. Rename source
files. Update Cargo.toml in lcp-server and workspace root to depend on `doppel`.
Also rename the `from_patterns_file` method to `from_secrets_file`.

## Prerequisites
- Step 01 committed locally in its-classified repo
- `[patch]` section in lcp `Cargo.toml` pointing to local its-classified checkout
  (set up during pre-flight)

## Files to Read Before Starting
- `Cargo.toml` (workspace root) — `its-classified` git dependency line
- `crates/lcp-server/Cargo.toml` — dependency declaration
- `crates/lcp-server/src/ext/scrub.rs` — `ScrubExt`, `ScrubExtLoadError`, module doc
- `crates/lcp-server/src/ext/sse_unscrub.rs` — `SseUnscrubStream`, `unscrub_non_sse`, `unscrub_sse`
- `crates/lcp-server/src/ext/mod.rs` — module declarations and re-exports
- `crates/lcp-server/src/lib.rs` — `pub use ext::{ScrubExt, ScrubExtLoadError}`

## Implementation

### Task 1: Update Cargo.toml dependencies

In `Cargo.toml` (workspace root), change:
```toml
its-classified = { git = "https://git.sr.ht/~mackenney/its-classified", features = ["async"] }
```
to:
```toml
doppel = { git = "https://git.sr.ht/~mackenney/its-classified", features = ["async"] }
```
Note: git URL stays the same; only the package name changes. The `[patch]`
section (set up in pre-flight) maps this to the local checkout.

In `crates/lcp-server/Cargo.toml`, change:
```toml
its-classified = { workspace = true }
```
to:
```toml
doppel = { workspace = true }
```

### Task 2: Rename source files
```sh
cd /home/ignacio/pr/llm-cache-proxy
git mv crates/lcp-server/src/ext/scrub.rs crates/lcp-server/src/ext/doppel.rs
git mv crates/lcp-server/src/ext/sse_unscrub.rs crates/lcp-server/src/ext/sse_restore.rs
```

### Task 3: Update ext/doppel.rs (was scrub.rs)

Imports:
- `use its_classified::scrub` → `use doppel::swap`
- `use its_classified::types::{Entry, Pattern, SessionKey}` → `use doppel::{Entry, Pattern, SessionKey}`
- Inline path (no `use`): `its_classified::PatternsFile` at `from_secrets_file` body → `doppel::SecretsFile`
- Inline path (no `use`): `its_classified::PatternsFileError` in `From` impl → `doppel::SecretsFileError`

Types and structs:
- `pub struct ScrubExt` → `pub struct DoppelExt`
- `pub enum ScrubExtLoadError` → `pub enum DoppelExtLoadError`
- `impl ScrubExt` → `impl DoppelExt`
- `impl Extension for ScrubExt` → `impl Extension for DoppelExt`
- `fn name() -> "scrub"` → `fn name() -> "doppel"`

Methods:
- `pub fn from_patterns_file(` → `pub fn from_secrets_file(`
- `fn on_upstream_body`: `scrub(&body, &patterns)` → `swap(&body, &patterns)`

Update the module-level `//!` doc comment:
- `ScrubExt` → `DoppelExt`
- `its_classified::scrub` → `doppel::swap`
- `UnscrubStream` → `RestoreStream`
- `scrub/unscrub` → `swap/restore`
- `scrubbing` → `swapping` / `unscrubbing` → `restoring`

Update inline `#[cfg(test)]` module:
- `use its_classified::` → `use doppel::`
- `ScrubExt::new(` → `DoppelExt::new(`
- Test function names: `phase2_scrubs_*` → `phase2_swaps_*`, `phase3_restores_*` stays
- `scrub(&body, ...)` in test assertions → update references

### Task 4: Update ext/sse_restore.rs (was sse_unscrub.rs)

Types:
- `pub struct SseUnscrubStream` → `pub struct SseRestoreStream`
- `impl SseUnscrubStream` → `impl SseRestoreStream`
- `impl Stream for SseUnscrubStream` → `impl Stream for SseRestoreStream`

Functions:
- `async fn unscrub_non_sse(` → `async fn restore_non_sse(`
- `async fn unscrub_sse(` → `async fn restore_sse(`

Imports:
- `use its_classified::unscrub_stream` → `use doppel::restore_stream`
- `use its_classified::types::{Entry, SessionKey}` → `use doppel::{Entry, SessionKey}`
- Any remaining `its_classified` imports → `doppel`

Internal calls:
- `unscrub_stream(inner, entries, session_key)` → `restore_stream(inner, entries, session_key)`

### Task 5: Update ext/mod.rs
```rust
pub mod doppel;       // was: pub mod scrub
pub mod sse_restore;  // was: pub mod sse_unscrub

pub use doppel::{DoppelExt, DoppelExtLoadError};  // was: ScrubExt, ScrubExtLoadError
```

### Task 6: Update src/lib.rs
```rust
pub use ext::{DoppelExt, DoppelExtLoadError};  // was: ScrubExt, ScrubExtLoadError
```

### Task 7: Build and test
```sh
cd /home/ignacio/pr/llm-cache-proxy
cargo build -p lcp-server
cargo nextest run -p lcp-server
```

### Task 8: Commit
```sh
git add -A
git commit -m "step-02: rename lcp-server ext layer to doppel"
```

## Acceptance Criteria

- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo build -p lcp-server` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run -p lcp-server` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -rE "ScrubExt|SseUnscrubStream|its_classified|its-classified" crates/lcp-server/src/ --include="*.rs"` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -E "ScrubExt|its-classified|its_classified" crates/lcp-server/Cargo.toml Cargo.toml` returns no matches (except the `[patch]` URL which still references the old repo name — that's expected)
- [ ] `test -f crates/lcp-server/src/ext/doppel.rs && test -f crates/lcp-server/src/ext/sse_restore.rs` exits 0
- [ ] `test ! -f crates/lcp-server/src/ext/scrub.rs && test ! -f crates/lcp-server/src/ext/sse_unscrub.rs` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep "from_secrets_file" crates/lcp-server/src/ext/doppel.rs` returns at least 1 match

## Reviewer Instructions

You are reviewing Step 02 implementation. Verify:

1. `cd /home/ignacio/pr/llm-cache-proxy && cargo build -p lcp-server` — must exit 0
2. `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run -p lcp-server` — must exit 0
3. `cd /home/ignacio/pr/llm-cache-proxy && grep -rE "ScrubExt|SseUnscrubStream|its_classified|its-classified" crates/lcp-server/src/ --include="*.rs"` — must return no matches
4. Check `crates/lcp-server/src/ext/doppel.rs` exists and exports `DoppelExt`, `DoppelExtLoadError`
5. Check `crates/lcp-server/src/ext/sse_restore.rs` exists and exports `SseRestoreStream`
6. Check `crates/lcp-server/src/lib.rs` exports `DoppelExt`, `DoppelExtLoadError`
7. Confirm `from_patterns_file` is renamed to `from_secrets_file`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit produced by this step.
