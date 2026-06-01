# Step 04: Rename Integration Tests

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` and update all consumers in lcp.

### Phase Context
Wave 2 — runs in parallel with step-03 after step-02 completes. This step
depends on step-02 because it imports `DoppelExt` from `lcp-server`.

### This Step
Rename `tests/integration/scrub.rs` → `tests/integration/doppel.rs` and update
all imports, type references, macro names, function names, and comments.
Update `tests/integration/mod.rs` and `tests/Cargo.toml`.

## Prerequisites
- Step 02 committed: `DoppelExt` exported from `lcp-server`, `doppel` in workspace deps

## Files to Read Before Starting
- `tests/integration/scrub.rs` — full file (~1070 lines)
- `tests/integration/mod.rs` — module declarations
- `tests/Cargo.toml` — dependency declarations

## Implementation

### Task 1: Rename the file
```sh
cd /home/ignacio/pr/llm-cache-proxy
git mv tests/integration/scrub.rs tests/integration/doppel.rs
```

### Task 2: Update tests/integration/mod.rs
```rust
mod doppel;   // was: mod scrub;
```

### Task 3: Update tests/Cargo.toml
```toml
doppel = { workspace = true }  # was: its-classified = { workspace = true }
```

### Task 4: Update imports in tests/integration/doppel.rs

Top-level imports:
```rust
use doppel::{register, patterns};            // was: use its_classified::{register, tier1::patterns}
use lcp_server::{ExtensionPipeline, DoppelExt};  // was: ScrubExt
```

Inside individual test functions that do direct `scrub` calls:
```rust
use doppel::swap as doppel_swap;  // was: use its_classified::scrub as ic_scrub
```

### Task 5: Rename macro and update all invocations

The macro `wire_scrub_test!` → `wire_doppel_test!`. It is defined once and
invoked 8 times. Find-replace both the definition and all invocations.

### Task 6: Update all type and function references

- `ScrubExt::new(...)` → `DoppelExt::new(...)`
- `ic_scrub(...)` → `doppel_swap(...)`
- `ScrubExt` → `DoppelExt` in all positions
- Test function names: `wire_anthropic_key_scrubbed` → `wire_anthropic_key_swapped`,
  `wire_openai_classic_key_scrubbed` → `wire_openai_classic_key_swapped`, etc.
- `unscrub_restores_secret_from_*` → `restore_returns_secret_from_*`
- `wire_multiple_tier1_secrets_all_scrubbed` → `wire_multiple_secrets_all_swapped`
- `wire_tier2_*_scrubbed` → `wire_registered_*_swapped`
- `wire_tier1_and_tier2_in_same_payload` → update name to remove tier terminology

### Task 7: Update comments and doc header

Update the module-level `//!` doc comment:
- `Scrub/unscrub integration tests` → `Doppel swap/restore integration tests`
- `ScrubExt` → `DoppelExt`
- `its-classified` → `doppel`
- `its_classified::scrub` → `doppel::swap`
- `its_classified::unscrub_stream` → `doppel::restore_stream`
- `scrub/unscrub extension` → `doppel extension`

Update inline comments:
- `"upstream body (tier1+tier2)"` → `"upstream body (pattern+registered)"`
- `"registered secret must be scrubbed"` → `"registered secret must be swapped"`
- `"client SSE response: scrubbed fake must not be visible"` → update
- All `tier1`/`tier2` terminology in comments → `pattern-based`/`registered`

### Task 8: Build and test
```sh
cd /home/ignacio/pr/llm-cache-proxy
cargo nextest run --test integration
```

### Task 9: Commit
```sh
git add -A
git commit -m "step-04: rename integration tests to doppel"
```

## Acceptance Criteria

- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run --test integration` exits 0
- [ ] `test -f tests/integration/doppel.rs` exits 0
- [ ] `test ! -f tests/integration/scrub.rs` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -nE "its_classified|its-classified|ScrubExt|ic_scrub|tier1::" tests/integration/doppel.rs` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep "mod doppel" tests/integration/mod.rs` returns a match
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep "its.classified" tests/Cargo.toml` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -c "DoppelExt" tests/integration/doppel.rs` returns a number > 0

## Reviewer Instructions

You are reviewing Step 04 implementation. Verify:

1. `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run --test integration` — must exit 0 with same test count as before
2. `cd /home/ignacio/pr/llm-cache-proxy && grep -nE "its_classified|ScrubExt|ic_scrub|tier1::" tests/integration/doppel.rs` — must return no matches
3. `test ! -f /home/ignacio/pr/llm-cache-proxy/tests/integration/scrub.rs` — must exit 0
4. `test -f /home/ignacio/pr/llm-cache-proxy/tests/integration/doppel.rs` — must exit 0
5. Check `tests/Cargo.toml` has `doppel` not `its-classified`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit produced by this step.
