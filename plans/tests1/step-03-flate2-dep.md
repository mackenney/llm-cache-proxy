# Step 03: Add flate2 Dev-Dependency

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 0: infrastructure extension. This step adds `flate2` as an explicit dev-dependency so that the compression spec test (Wave 1, step-10) can build a gzip-encoded request body in test code. All Wave 0 steps are independent and run in parallel.

### This Step
`flate2` is already present transitively (via `tower-http`'s `decompression-full` feature), but adding it explicitly to `tests/Cargo.toml` pins the API and makes the dependency explicit for anyone reading the test crate. Without it, `use flate2::write::GzEncoder` would not resolve in test source.

## Prerequisites
- None (Wave 0 step)

## Files to Read Before Starting
- `tests/Cargo.toml` — confirm current deps and where to add the new line

## Implementation

### Task 1: Add flate2 under [dependencies] in tests/Cargo.toml

Open `tests/Cargo.toml`. In the `[dependencies]` section, add:

```toml
flate2 = "1"
```

The final `[dependencies]` block should look like:

```toml
[dependencies]
lcp-core = { workspace = true }
lcp-server = { workspace = true }
tokio = { workspace = true }
reqwest = { workspace = true }
serde_json = { workspace = true }
futures-util = { workspace = true }
axum = { workspace = true }
bytes = { workspace = true }
tracing = { workspace = true }
flate2 = "1"
```

No `workspace = true` for flate2 — it is not declared in the workspace-level Cargo.toml. Use a direct version specifier.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build --test spec` exits 0
- [ ] `cargo build --test integration` exits 0
- [ ] `grep 'flate2' tests/Cargo.toml` outputs `flate2 = "1"`
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)

## Reviewer Instructions

You are reviewing Step 03 implementation. Verify:

1. Run `grep 'flate2' tests/Cargo.toml` — must output `flate2 = "1"`
2. Run `cargo build --test spec` — must exit 0
3. Run `cargo nextest run --test spec` — must exit 0 (no regressions)

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: remove the `flate2 = "1"` line from `tests/Cargo.toml`.
