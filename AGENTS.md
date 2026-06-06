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
MASTER_PROGRESS.md  Single source of truth for project-wide work status
```

## Plans

- `plans/` is committed; plans are **deleted** once execution is complete.
- Plans are created by the planner skill; names follow `plans/<feature>/PROGRESS.md` + `plans/<feature>/step-NN-*.md`.
- Code comments and docs MUST NOT reference plan files.

## Master Progress

`MASTER_PROGRESS.md` is the single source of truth for project-wide work status.
Every human and agent working on lcp reads it first to understand current state.

**What it contains:**
- **In Progress** — active plans with a link to their plan directory
- **Queued** — not-started plans with a link to their plan directory
- **Completed** — one-liner per finished feature/plan, with commit hash
- **Known Gaps** — identified issues with no active plan owner

**Rules:**
- When a plan completes and is deleted → add a one-liner to Completed with the merge commit
- When a new plan is created → add it to Queued
- When work starts on a plan → move it from Queued to In Progress
- Keep entries as one-liners; all detail lives in plan files and git history
- Never let `MASTER_PROGRESS.md` drift: update it in the same commit as the plan change

## SPEC-driven development

- Every implementation must be driven by a spec.
- Specs live at `SPEC.md` (repo root) or `crates/*/SPEC.md` (per-crate).
- Spec documents use MUST / SHOULD / MAY language (RFC 2119).
- The pipeline is: **investigate → spec → plan → orchestrate**.
- No code is written before a spec exists for the component being built.

## Artifacts

- All transient working files (investigation scans, fact-check reports,
  progress trackers, brainstorm notes) live in `artifacts/` and are git-ignored.
- Never commit files from `artifacts/`.
- Never commit scratch files, session recordings, or API fixture files to the repo root.

## Testing

- **Inline unit tests** (`#[cfg(test)]` modules in `src/`) are implementation details.
  Change them freely during refactors — they carry no external obligation.
- **External tests** (`tests/`) are behavioral contracts.
  A failing external test is a bug or a deliberate spec change, never a refactor casualty.
- Three external tiers: **spec invariants** (`tests/spec/`) on every commit; **integration** (`tests/integration/`); **E2E** (`tests/e2e/`) gated by `--features test-e2e`.
- Never weaken an external test to make a refactor pass.
- Write spec invariant tests alongside the spec, before the implementation.
- Run: `cargo nextest run` (all); `cargo nextest run --test spec` (invariants only).

## Crates

| Crate | Role |
|---|---|
| `lcp-core` | Types, cache key hashing, SQLite cache, provider enum |
| `lcp-server` | Axum HTTP server, proxy handler, stats endpoints, extension pipeline |
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
- **Tests:** `cargo nextest run`

## Publishing

To publish a new release to crates.io:

1. `cargo nextest run` — all tiers must pass
2. `cargo clippy --workspace --all-targets -- -D warnings` — must be clean
3. Run an E2E test against a real provider: `cargo nextest run --test e2e --features test-e2e`
4. Bump the workspace version in the root `Cargo.toml`
5. `cargo build` — verifies `Cargo.lock` is updated
6. Commit: `chore: bump version to vX.Y.Z`
7. Tag the commit: `git tag vX.Y.Z <commit-hash>`
8. `cargo publish -p lcp-core && cargo publish -p lcp-server && cargo publish -p lcp`
   (publish in dependency order; each may need a moment before the next is accepted)
