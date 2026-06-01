# Step 03: Rename lcp CLI Config and References

## Context

### Overall Objective
Rename the `its-classified` library to `doppel` and update all consumers in lcp.

### Phase Context
Wave 2 — runs in parallel with step-04 after step-02 completes. This step
depends on step-02 because it imports `DoppelExt` from `lcp-server`.

### This Step
Update the lcp binary: rename `ScrubConfig` → `DoppelConfig`, the TOML config
key `[extensions.scrub]` → `[extensions.doppel]`, `patterns_file` → `secrets_file`,
and all user-facing log/warning messages. Add backward compatibility via serde
aliases so existing config files continue to work.

## Prerequisites
- Step 02 committed: `DoppelExt` exported from `lcp-server`

## Files to Read Before Starting
- `crates/lcp/src/main.rs` — full file; all changes are here
- `~/.config/lcp/config.toml` — actual user config to understand what needs backward compat

## Implementation

### Task 1: Update imports
```rust
use lcp_server::{ExtensionPipeline, DoppelExt, ServerConfig, serve};
// was: ScrubExt
```

### Task 2: Rename config struct with backward compat

```rust
#[derive(Deserialize, Default)]
struct ExtensionsConfig {
    #[serde(alias = "scrub")]
    doppel: Option<DoppelConfig>,
}

/// Configuration for the built-in doppel extension.
///
/// ```toml
/// [extensions.doppel]
/// secrets_file = "~/.config/lcp/secrets.toml"
/// ```
///
/// Create a secrets file with `doppel init <path>`.
/// Register secrets with `doppel secret add <value>`.
#[derive(Deserialize, Default)]
struct DoppelConfig {
    #[serde(alias = "patterns_file")]
    secrets_file: Option<String>,
}
```

The `#[serde(alias = "scrub")]` means existing `[extensions.scrub]` config files
still deserialize into the `doppel` field. Similarly, `#[serde(alias = "patterns_file")]`
preserves backward compat for the old field name.

### Task 3: Update build_extension_pipeline function

All references to `scrub_cfg`, `scrub`, `ScrubExt`, and `patterns_file`:
- `&ext.doppel` (was `&ext.scrub`)
- `doppel_cfg` (was `scrub_cfg`)
- `&doppel_cfg.secrets_file` (was `&scrub_cfg.patterns_file`)
- `DoppelExt::from_secrets_file(...)` (was `ScrubExt::from_patterns_file`)

### Task 4: Update log messages and user-facing strings

Replace all occurrences:
- `"scrub extension loaded"` → `"doppel extension loaded"`
- `"[extensions.scrub]"` → `"[extensions.doppel]"`
- `"its-classified init <path>"` → `"doppel init <path>"`
- `"its-classified register --patterns <path>"` → `"doppel secret add <value> --secrets-file <path>"`
- `"scrub patterns file could not be loaded"` → `"doppel secrets file could not be loaded"`
- `"patterns_file"` (in user messages) → `"secrets_file"`
- `"scrubbing is disabled"` → `"doppel extension is disabled"`
- `"scrub extension"` → `"doppel extension"` (in all log messages)

### Task 5: Verify Cargo.toml

`crates/lcp/Cargo.toml` does not depend on `its-classified` directly (it goes
through `lcp-server`). Verify no stray references exist.

### Task 6: Build and test
```sh
cd /home/ignacio/pr/llm-cache-proxy
cargo build -p lcp
cargo nextest run -p lcp
```

### Task 7: Commit
```sh
git add -A
git commit -m "step-03: rename lcp CLI config to doppel"
```

## Acceptance Criteria

- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo build -p lcp` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run -p lcp` exits 0
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep -rE "ScrubExt|ScrubConfig|its.classified|its_classified" crates/lcp/src/ --include="*.rs"` returns no matches
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'serde.*alias.*scrub' crates/lcp/src/main.rs` returns at least 1 match (backward compat for `[extensions.scrub]`)
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'serde.*alias.*patterns_file' crates/lcp/src/main.rs` returns at least 1 match (backward compat for `patterns_file`)
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'DoppelConfig' crates/lcp/src/main.rs` returns at least 1 match
- [ ] `cd /home/ignacio/pr/llm-cache-proxy && grep 'from_secrets_file' crates/lcp/src/main.rs` returns at least 1 match

## Reviewer Instructions

You are reviewing Step 03 implementation. Verify:

1. `cd /home/ignacio/pr/llm-cache-proxy && cargo build -p lcp` — must exit 0
2. `cd /home/ignacio/pr/llm-cache-proxy && cargo nextest run -p lcp` — must exit 0
3. `cd /home/ignacio/pr/llm-cache-proxy && grep -rE "ScrubExt|ScrubConfig|its.classified|patterns_file[^\"]\b" crates/lcp/src/main.rs` — must return no matches (note: `patterns_file` may appear inside the serde alias string, that's fine)
4. Check `crates/lcp/src/main.rs` contains `#[serde(alias = "scrub")]` and `#[serde(alias = "patterns_file")]`
5. Check log messages reference `doppel` not `scrub` or `its-classified`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert` the single commit produced by this step.
