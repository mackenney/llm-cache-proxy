# Step 01: Add `body_limit_bytes` to ServerConfig

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 0 modifies lcp-server first because the CLI and test harness both depend on `ServerConfig`'s shape. This step and step-02 can run in parallel since they touch different files.

### This Step
Add the `body_limit_bytes: u64` field to `ServerConfig` in `crates/lcp-server/src/server.rs`. This field holds the maximum incoming request body size in bytes; 0 means no limit.

## Prerequisites
- None — this is a Wave 0 step.

## Files to Read Before Starting
- `crates/lcp-server/src/server.rs:14-33` — current `ServerConfig` struct definition

## Implementation

### Task 1: Add `body_limit_bytes` field to `ServerConfig`

In `crates/lcp-server/src/server.rs`, add a new field to `ServerConfig` after `timeout_seconds`:

```rust
/// Maximum incoming request body size in bytes. `0` means no limit.
pub body_limit_bytes: u64,
```

Insert this immediately after line 20 (`pub timeout_seconds: u64,`).

**Field placement rationale:** Keep related numeric config fields (`timeout_seconds`, `body_limit_bytes`) adjacent before the optional URL overrides.

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo build -p lcp-server` exits with code 0
- [ ] `cargo clippy -p lcp-server -- -D warnings` exits with code 0
- [ ] `grep -n 'body_limit_bytes: u64' crates/lcp-server/src/server.rs` outputs a line number (field exists)

## Reviewer Instructions

You are reviewing Step 01. Verify:
1. Run `cargo build -p lcp-server` — must exit 0
2. Run `cargo clippy -p lcp-server -- -D warnings` — must exit 0
3. Check `crates/lcp-server/src/server.rs` contains `pub body_limit_bytes: u64` with doc comment explaining 0 = no limit

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-01: server-config" and revert it.
