# Step 03: Add `--body-limit` CLI Flag

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 1 adds the CLI interface. Depends on Wave 0 (step-01) completing so `ServerConfig` has the `body_limit_bytes` field.

### This Step
Add `--body-limit` / `LCP_BODY_LIMIT` CLI flag to `crates/lcp/src/main.rs`:
1. Add field to `Cli` struct
2. Add field to `FileConfig` struct
3. Add `seed!` macro call in `seed_env_from_config_file`
4. Add line in `print_config`
5. Wire to `ServerConfig` construction

## Prerequisites
- Step 01 complete (`ServerConfig` has `body_limit_bytes: u64`)

## Files to Read Before Starting
- `crates/lcp/src/main.rs:43-66` — `Cli` struct with `timeout` pattern
- `crates/lcp/src/main.rs:68-82` — `FileConfig` struct
- `crates/lcp/src/main.rs:154-177` — `seed_env_from_config_file` macro calls
- `crates/lcp/src/main.rs:193-246` — `print_config` function
- `crates/lcp/src/main.rs:360-370` — `ServerConfig` construction

## Implementation

### Task 1: Add `body_limit` field to `Cli` struct

After the `timeout` field (line 45), add:

```rust
/// Maximum incoming request body in bytes. 0 means no limit.
#[arg(long, env = "LCP_BODY_LIMIT", default_value = "104857600")]
body_limit: u64,
```

### Task 2: Add `body_limit` field to `FileConfig` struct

After the `timeout` field (line 76), add:

```rust
body_limit: Option<u64>,
```

### Task 3: Add `seed!` call in `seed_env_from_config_file`

After the `seed!("LCP_TIMEOUT", fc.timeout);` line (line 170), add:

```rust
seed!("LCP_BODY_LIMIT", fc.body_limit);
```

### Task 4: Add line in `print_config`

After the `println!("timeout = {}", cli.timeout);` line (line 208), add:

```rust
println!("body_limit = {}", cli.body_limit);
```

### Task 5: Wire to `ServerConfig` construction

In the `ServerConfig { ... }` block (around line 360-370), add after `timeout_seconds`:

```rust
body_limit_bytes: cli.body_limit,
```

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo build -p lcp` exits with code 0
- [ ] `cargo clippy -p lcp -- -D warnings` exits with code 0
- [ ] `cargo run -p lcp -- --print-config 2>&1 | grep 'body_limit = 104857600'` outputs a line (default appears in print-config)
- [ ] `LCP_BODY_LIMIT=50000000 cargo run -p lcp -- --print-config 2>&1 | grep 'body_limit = 50000000'` outputs a line (env var works)
- [ ] `cargo run -p lcp -- --body-limit 25000000 --print-config 2>&1 | grep 'body_limit = 25000000'` outputs a line (CLI flag works)

## Reviewer Instructions

You are reviewing Step 03. Verify:
1. Run `cargo build -p lcp` — must exit 0
2. Run `cargo clippy -p lcp -- -D warnings` — must exit 0
3. Run `cargo run -p lcp -- --print-config 2>&1 | grep 'body_limit'` — must show `body_limit = 104857600`
4. Check `crates/lcp/src/main.rs`:
   - `Cli` struct has `body_limit: u64` with `#[arg(long, env = "LCP_BODY_LIMIT", default_value = "104857600")]`
   - `FileConfig` struct has `body_limit: Option<u64>`
   - `seed!("LCP_BODY_LIMIT", fc.body_limit);` call exists
   - `print_config` prints `body_limit`
   - `ServerConfig` construction includes `body_limit_bytes: cli.body_limit`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-03: cli-flag" and revert it.
