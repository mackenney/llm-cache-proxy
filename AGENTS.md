# Agent Guidelines — lcp

## Project

`lcp` (llm-cache-proxy) is a local HTTP proxy that caches LLM API calls on
disk and replays them on cache hits. Implementation is SPEC-driven: every
non-trivial module or behavior must have a corresponding spec document before
code is written.

## Workspace layout

```
crates/             Rust workspace crates
  lcp-core/         Types, hashing, SQLite cache, provider config
  lcp-server/       Axum HTTP server, proxy routing, SSE passthrough
  lcp/              CLI binary
plans/              Active development plans — committed, deleted when complete
references/         External reference projects — git-ignored, never committed
artifacts/          Transient working files — git-ignored, never committed
SPEC.md             Behavioral contract for the full system
TESTING.md          Permanent testing strategy
progress.md         Current work status
```

## Plans

- `plans/` is committed; plans are **deleted** once execution is complete.
- Plans are created by the planner skill; names follow `plans/<feature>/PROGRESS.md`.
- Code comments and docs MUST NOT reference plan files.

## SPEC-driven development

- Every implementation must be driven by a spec.
- Specs live at `SPEC.md` (repo root) or `crates/*/SPEC.md` (per-crate).
- Spec documents use MUST / SHOULD / MAY language (RFC 2119).
- The pipeline is: **investigate → spec → plan → orchestrate**.
- No code is written before a spec exists for the component being built.

## Testing

See `TESTING.md` for the full strategy. Rules for agents:

- **Inline unit tests** (`#[cfg(test)]` in `src/`) are implementation details.
  Change them freely during refactors — they carry no external obligation.
- **External tests** (`tests/`) are behavioral contracts.
  A failing external test is a bug or a deliberate spec change, never a refactor side effect.
- External tiers in order of scope:
  1. **Spec invariants** (`tests/spec/`) — direct MUST/SHOULD assertions; must pass on every commit
  2. **Integration** (`tests/integration/`) — public-API behavior
  3. **E2E** (`tests/e2e/`) — real upstream calls, gated by `--features test-e2e`
- Write spec invariant tests alongside the spec, before the implementation.
- Never weaken an external test to make a refactor pass.

## Crates

| Crate | Role |
|---|---|
| `lcp-core` | Types, cache key hashing, SQLite cache, provider enum |
| `lcp-server` | Axum HTTP server, proxy handler, stats endpoints |
| `lcp` | CLI binary |

## Conventions

- Commit messages: imperative mood, 72 chars, no period
- Branch names: `ignacio@<crate-or-feature>/<kebab-description>`
- Format with `cargo fmt`, lint with `cargo clippy` before committing
- No `#[allow(clippy::*)]` without a comment explaining why
- No section-separator comments (`// ---`, `// ===`, etc.)
- Comments explain WHY, not WHAT

## Tooling

- **Shell:** bash; `jq`, `rg`, `fdfind` available
- **Build:** `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt`
- **Tests:** `cargo nextest run` (preferred); see `TESTING.md` for per-tier commands
