# Step 01: Rename Library (its-classified → doppel)

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` with updated API vocabulary
(`swap`/`restore`, `patterns` module, `SecretsFile`, etc.). No behavior changes.

### Phase Context
Wave 0 — must complete before any lcp-side steps. The library is a git
dependency; lcp must point to the renamed crate before it can compile.

### This Step
All changes live in the `its-classified` source repo at
`/home/ignacio/pr/its-classified`. This is the largest step: renames the crate
identity, all public API symbols, CLI commands, env vars, source files, tests,
and repo-level docs in a single commit.

## Prerequisites
- Repo is on a feature branch `ignacio@doppel/rename-crate` with a clean working tree.

## Files to Read Before Starting
- `Cargo.toml` — package name, lib name, workspace members
- `cli/Cargo.toml` — CLI crate name, binary name, dependency on `its-classified`
- `src/lib.rs` — all `pub(crate) mod` and `pub use` declarations; the docstring quick-start example
- `src/scrub.rs` — `pub fn scrub`
- `src/unscrub.rs` — `pub fn unscrub`
- `src/unscrub_stream.rs` — `pub fn unscrub_stream`, `pub struct UnscrubStream`
- `src/unscrub_core.rs` — internal helpers
- `src/types.rs` — `ScrubResult`, `ScrubError`, `Entry`, `SessionKey`; `pub use crate::tier1::Pattern`
- `src/tier1.rs` — `pub mod patterns { ... }` inner module with factory fns, `Tier1Def`, `Pattern` enum
- `src/tier2.rs` — `register`, `register_with_options`, `RegistrationOptions`, `RegistrationError`, `Tier2Pat`
- `src/patterns_file.rs` — `PatternsFile`, `PatternsFileError`, `Tier1Entry`, `Tier2Entry`
- `cli/src/main.rs` — `Commands::Scrub`, `Commands::Unscrub`, `ITS_CLASSIFIED_KEY`, `INIT_COMMENT_BLOCK`
- `SPEC.md` — INV-20 references `ITS_CLASSIFIED_KEY`
- `AGENTS.md` — test commands reference `its-classified`

## Complete Rename Table

### Functions
| Old | New |
|-----|-----|
| `scrub` | `swap` |
| `unscrub` | `restore` |
| `unscrub_stream` | `restore_stream` |
| `register` | `register` (unchanged) |
| `register_with_options` | `register_with_options` (unchanged) |
| `run_scrub` (CLI) | `run_swap` |
| `run_unscrub` (CLI) | `run_restore` |

### Types
| Old | New |
|-----|-----|
| `ScrubResult` | `SwapResult` |
| `ScrubError` | `SwapError` |
| `UnscrubStream` | `RestoreStream` |
| `UnscrubError` | `RestoreError` |
| `RegistrationOptions` | `SecretOptions` |
| `RegistrationError` | `SecretError` |
| `PatternsFile` | `SecretsFile` |
| `PatternsFileError` | `SecretsFileError` |
| `Tier1Entry` | `PatternEntry` |
| `Tier2Entry` | `SecretEntry` |
| `Pattern`, `Entry`, `SessionKey`, `Tier1Def`, `Tier2Pat` | unchanged |

### Modules
| Old | New |
|-----|-----|
| `tier1` (file: `tier1.rs`) | `patterns` (file: `patterns.rs`) |
| `tier1::patterns` (inner module) | **FLATTENED** — factory fns become top-level in `patterns.rs` |
| `tier2` (file: `tier2.rs`) | `secrets` (file: `secrets.rs`) |
| `patterns_file` | `secrets_file` |

### Source Files
| Old | New |
|-----|-----|
| `src/scrub.rs` | `src/swap.rs` |
| `src/unscrub.rs` | `src/restore.rs` |
| `src/unscrub_core.rs` | `src/restore_core.rs` |
| `src/unscrub_stream.rs` | `src/restore_stream.rs` |
| `src/tier1.rs` | `src/patterns.rs` |
| `src/tier2.rs` | `src/secrets.rs` |
| `src/patterns_file.rs` | `src/secrets_file.rs` |

### CLI
| Old | New |
|-----|-----|
| `Commands::Scrub` | `Commands::Swap` |
| `Commands::Unscrub` | `Commands::Restore` |
| `ITS_CLASSIFIED_KEY` env var | `DOPPEL_KEY` |
| `#[command(name = "its-classified")]` | `#[command(name = "doppel")]` |
| binary name: `its-classified` | `doppel` |
| crate name: `its-classified-cli` | `doppel-cli` |

### Cargo.toml
```toml
# Root Cargo.toml
[package]
name = "doppel"          # was: its-classified

[lib]
name = "doppel"          # was: its_classified

# cli/Cargo.toml
[package]
name = "doppel-cli"      # was: its-classified-cli

[[bin]]
name = "doppel"          # was: its-classified

[dependencies]
doppel = { path = ".." } # was: its-classified = { path = ".." }
```

## Implementation

### Task 1: Cargo.toml identity changes

In root `Cargo.toml`:
- `name = "doppel"` (package)
- `name = "doppel"` (lib)

In `cli/Cargo.toml`:
- `name = "doppel-cli"` (package)
- `name = "doppel"` (bin)
- `doppel = { path = ".." }` (dependency)

### Task 2: Rename source files (git mv)

Execute all renames before editing content to preserve git rename detection:
```sh
cd /home/ignacio/pr/its-classified
git mv src/scrub.rs src/swap.rs
git mv src/unscrub.rs src/restore.rs
git mv src/unscrub_core.rs src/restore_core.rs
git mv src/unscrub_stream.rs src/restore_stream.rs
git mv src/tier1.rs src/patterns.rs
git mv src/tier2.rs src/secrets.rs
git mv src/patterns_file.rs src/secrets_file.rs
```

### Task 3: Update src/lib.rs

Replace module declarations and re-exports. **Critical: the `patterns` module
must NOT produce a `doppel::patterns::patterns::*` stutter.**

Current state:
```rust
pub mod tier1;
pub use tier1::patterns;  // re-exports the inner `pub mod patterns { ... }` from tier1.rs
```

After rename, `src/patterns.rs` (was `tier1.rs`) contains `pub mod patterns { ... }`.
If `lib.rs` says `pub mod patterns;` this creates `doppel::patterns::patterns::anthropic()`.

**Fix:** Flatten the inner `pub mod patterns { ... }` block in `src/patterns.rs`.
Move all factory functions (`anthropic()`, `all()`, etc.) to the file's top level.
Remove the inner module wrapper entirely. Then in `lib.rs`:

```rust
pub mod patterns;       // was: pub mod tier1; pub use tier1::patterns;
```

Callers use `doppel::patterns::anthropic()` — no stutter.

Full `lib.rs` re-exports after the change:
```rust
pub(crate) mod crypto;
pub(crate) mod fake;
pub mod secrets_file;   // was: patterns_file
pub(crate) mod swap;    // was: scrub
pub mod segment;
pub(crate) mod serde_helpers;
pub mod patterns;       // was: pub mod tier1; pub use tier1::patterns;
pub(crate) mod secrets; // was: tier2
pub mod types;
pub(crate) mod restore;       // was: unscrub
pub(crate) mod restore_core;  // was: unscrub_core
#[cfg(feature = "async")]
pub(crate) mod restore_stream; // was: unscrub_stream

pub use secrets_file::{SecretsFile, SecretsFileError, PatternEntry, SecretEntry};
pub use swap::swap;
pub use patterns::Pattern;
pub use secrets::{SecretError, SecretOptions, register, register_with_options};
pub use types::{Entry, SwapResult, SwapError, SessionKey};
pub use restore::{RestoreError, restore};
#[cfg(feature = "async")]
pub use restore_stream::{RestoreStream, restore_stream};
```

### Task 4: Flatten `pub mod patterns` in src/patterns.rs

In `src/patterns.rs` (was `tier1.rs`), the inner module:
```rust
pub mod patterns {
    use super::*;
    // ... factory fns: anthropic(), all(), etc.
}
```

Remove the `pub mod patterns { ... }` wrapper. Move all factory functions
(`anthropic()`, `openai_classic()`, `all()`, etc.) and their helper
`random_salt()` to the top level of `src/patterns.rs`. Keep `Tier1Def`,
`Pattern` enum, `match_segments`, `all_defs`, and the static `*_DEF` constants
at the top level (they're already at the file top level outside the inner
module).

### Task 5: Update src/types.rs

- `ScrubResult` → `SwapResult`
- `ScrubError` → `SwapError` (including variant names in `From` impls)
- `pub use crate::tier1::Pattern;` → `pub use crate::patterns::Pattern;`

### Task 6: Update src/swap.rs (was scrub.rs)

- Rename `pub fn scrub(` → `pub fn swap(`
- Update all internal references to `ScrubResult`, `ScrubError` → `SwapResult`, `SwapError`
- Update module paths: `crate::tier1::` → `crate::patterns::`

### Task 7: Update src/restore.rs and src/restore_core.rs

- `pub fn unscrub(` → `pub fn restore(`
- `UnscrubError` → `RestoreError`
- Update all internal cross-references

### Task 8: Update src/restore_stream.rs

- `pub struct UnscrubStream` → `pub struct RestoreStream`
- `pub fn unscrub_stream(` → `pub fn restore_stream(`
- Update all internal references

### Task 9: Update src/secrets.rs (was tier2.rs)

- `RegistrationOptions` → `SecretOptions`
- `RegistrationError` → `SecretError`

### Task 10: Update src/secrets_file.rs (was patterns_file.rs)

- `PatternsFile` → `SecretsFile`
- `PatternsFileError` → `SecretsFileError`
- `Tier1Entry` → `PatternEntry`
- `Tier2Entry` → `SecretEntry`
- **CRITICAL: Preserve TOML serialization keys.** The on-disk format uses
  `[[tier1]]` and `[[tier2]]` TOML keys. Add serde attributes:
  ```rust
  pub struct SecretsFile {
      // ...
      #[serde(rename = "tier1")]
      pub tier1: Vec<PatternEntry>,  // field can keep name tier1 or rename with serde alias
      #[serde(rename = "tier2")]
      pub tier2: Vec<SecretEntry>,
  }
  ```
  The field names in Rust can be `patterns`/`secrets` or stay `tier1`/`tier2`
  — as long as the serde `rename` preserves the TOML keys. Simplest: keep the
  field names as `tier1`/`tier2` and only rename the type names.
- Update internal references: `crate::tier1::` → `crate::patterns::`

### Task 11: Update remaining src/ files

Files `crypto.rs`, `fake.rs`, `segment.rs`, `serde_helpers.rs` may reference
old module paths or types. Run:
```sh
grep -rn "scrub\|unscrub\|tier1\|tier2\|ScrubResult\|ScrubError\|UnscrubError\|PatternsFile" src/ --include="*.rs"
```
Update every match.

### Task 12: Update all inline #[cfg(test)] blocks in src/

Each source file may have inline unit tests with old names. These are internal
tests (mutable per AGENTS.md) but should be renamed for consistency.

### Task 13: Update CLI source (cli/src/main.rs)

- `Commands::Scrub { .. }` → `Commands::Swap { .. }`
- `Commands::Unscrub { .. }` → `Commands::Restore { .. }`
- `fn run_scrub(...)` → `fn run_swap(...)`
- `fn run_unscrub(...)` → `fn run_restore(...)`
- `#[command(name = "its-classified")]` → `#[command(name = "doppel")]`
- `about = "Secret scrubbing for LLM request/response cycles"` → update to match new vocabulary
- `std::env::var("ITS_CLASSIFIED_KEY")` → `std::env::var("DOPPEL_KEY")`
- All error messages referencing `ITS_CLASSIFIED_KEY` → `DOPPEL_KEY`
- `INIT_COMMENT_BLOCK`: `its-classified register` → `doppel register`, `its-classified define` → `doppel define`
- `"its-classified init --patterns {}"` → `"doppel init --patterns {}"`
- All `use its_classified::` → `use doppel::`
- All old type/fn names in imports

### Task 14: Update external test files

- `tests/spec/inv.rs`: ~107 `scrub`/`unscrub` references, ~23 `its_classified` references
- `tests/integration/round_trip.rs`: ~64 `scrub`/`unscrub`, ~4 `its_classified`
- `tests/spec.rs`, `tests/integration.rs`: entry points, update if they reference old names
- All `use its_classified::` → `use doppel::`
- Test function names: update `scrub`→`swap`, `unscrub`→`restore` in names (keep INV numbers)

### Task 15: Update CLI test files

- `cli/tests/e2e/cli.rs`: `env!("CARGO_BIN_EXE_its-classified")` → `env!("CARGO_BIN_EXE_doppel")`
- `"scrub"` subcommand args → `"swap"`, `"unscrub"` → `"restore"`
- `ITS_CLASSIFIED_KEY` env references → `DOPPEL_KEY`
- `its-classified-e2e-` tmp file prefix → `doppel-e2e-`
- `cli/tests/cli_spec/inv_cli.rs` and `inv_management.rs`: same pattern
- All `use its_classified::` → `use doppel::`

### Task 16: Update repo-level documentation

- `SPEC.md`: all references to old names, INV-20/INV-21 (`ITS_CLASSIFIED_KEY`, `scrub`/`unscrub` CLI commands)
- `AGENTS.md`: test commands `cargo nextest run -p its-classified` → `cargo nextest run -p doppel`
- `README.md`: all example code, type names, function names
- `MASTER_PROGRESS.md`: project name references
- `src/lib.rs` lines 1-46: Quick Start docstring example uses old names

### Task 17: Build and test
```sh
cd /home/ignacio/pr/its-classified
cargo build --workspace
cargo nextest run --workspace
cargo clippy --workspace
```

### Task 18: Commit
```sh
git add -A
git commit -m "rename crate to doppel"
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd /home/ignacio/pr/its-classified && cargo build --workspace` exits 0
- [ ] `cd /home/ignacio/pr/its-classified && cargo nextest run --workspace` exits 0
- [ ] `cd /home/ignacio/pr/its-classified && cargo clippy --workspace` exits 0
- [ ] `cd /home/ignacio/pr/its-classified && grep -rE "its.classified|its_classified|ScrubResult|ScrubError|UnscrubStream|UnscrubError|PatternsFile|PatternsFileError|Tier1Entry|Tier2Entry|RegistrationOptions|RegistrationError|ITS_CLASSIFIED_KEY" src/ cli/src/ tests/ cli/tests/ --include="*.rs" | grep -v target` returns no matches
- [ ] `cd /home/ignacio/pr/its-classified && grep -rE "pub fn scrub\b|pub fn unscrub\b|pub fn unscrub_stream\b" src/ --include="*.rs"` returns no matches
- [ ] `cd /home/ignacio/pr/its-classified && grep "^name" Cargo.toml` outputs `name = "doppel"`
- [ ] `cd /home/ignacio/pr/its-classified && grep "^name" cli/Cargo.toml` outputs `name = "doppel-cli"`
- [ ] `cd /home/ignacio/pr/its-classified && grep 'pub mod patterns' src/patterns.rs` returns NO matches (inner module flattened)
- [ ] `cd /home/ignacio/pr/its-classified && grep 'DOPPEL_KEY' cli/src/main.rs` returns at least 1 match
- [ ] Existing secrets file at `~/.config/lcp/secrets.toml` still deserializes: `cd /home/ignacio/pr/its-classified && cargo test -p doppel --test spec` exits 0

## Reviewer Instructions

You are reviewing Step 01 implementation. Verify:

1. `cd /home/ignacio/pr/its-classified && cargo build --workspace` — must exit 0
2. `cd /home/ignacio/pr/its-classified && cargo nextest run --workspace` — must exit 0
3. `cd /home/ignacio/pr/its-classified && cargo clippy --workspace` — must exit 0
4. `cd /home/ignacio/pr/its-classified && grep -rE "its.classified|its_classified|ScrubResult|UnscrubStream|PatternsFile|Tier1Entry|RegistrationOptions|ITS_CLASSIFIED_KEY" src/ cli/src/ tests/ cli/tests/ --include="*.rs" | grep -v target` — must return no matches
5. `cd /home/ignacio/pr/its-classified && grep "^name" Cargo.toml` — must output `name = "doppel"`
6. `cd /home/ignacio/pr/its-classified && grep 'pub mod patterns' src/patterns.rs` — must return NO matches (stutter eliminated)
7. Confirm `src/lib.rs` exports: `swap`, `restore`, `restore_stream`, `RestoreStream`, `register`, `register_with_options`, `SecretOptions`, `SecretError`, `SecretsFile`, `SecretsFileError`, `PatternEntry`, `SecretEntry`, `SwapResult`, `SwapError`, `Entry`, `Pattern`, `SessionKey`, `patterns` module

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit produced by this step.
