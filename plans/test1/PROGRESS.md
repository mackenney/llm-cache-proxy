# PROGRESS.md

## Status
Complete

## Objective
Create external test infrastructure: MockUpstream HTTP server utility, TestHarness for test setup, and Priority 1 spec invariant tests for cache hit/miss behavior.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 1 | step-01, step-02 | Yes | — |
| 2 | step-03 | No | Wave 1 |
| 3 | step-04 | No | step-03 |
| 4 | step-05 | No | step-04 |

## Dependency Table

| Step | Depends On | Output |
|------|------------|--------|
| step-01 | — | `Cargo.toml` with `[features]`, `[[test]]`, `[dev-dependencies]` |
| step-02 | — | `tests/` directory structure with module files |
| step-03 | step-01, step-02 | `tests/common/mock_upstream.rs` — functional MockUpstream |
| step-04 | step-03 | `tests/common/harness.rs` — TestHarness wiring mock + proxy |
| step-05 | step-04 | `tests/spec/cache_hit.rs`, `tests/spec/cache_miss.rs` — 10 tests passing |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent
4. After each step: run `cargo nextest run` to confirm no regressions
5. Mark step complete only after reviewer passes
6. Advance to next wave only when all steps in current wave are complete
7. Blockers: stop and report to user with full context

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Steps
- [x] [step-01-cargo-config](./step-01-cargo-config.md) — Add test features, targets, dev-deps to Cargo.toml
- [x] [step-02-directory-structure](./step-02-directory-structure.md) — Create tests/ directory tree with module scaffolding
- [x] [step-03-mock-upstream](./step-03-mock-upstream.md) — Implement MockUpstream axum-based HTTP mock server
- [x] [step-04-test-harness](./step-04-test-harness.md) — Implement TestHarness wiring MockUpstream with proxy server
- [x] [step-05-spec-cache-tests](./step-05-spec-cache-tests.md) — Implement Priority 1 spec invariant tests (cache hit/miss)

## Open Decisions
None — all conflicts resolved in synthesis.

## Resolution Notes
- **MockUpstream location**: `tests/common/` (not `tests/support/`) per synthesis directive — utility module, not a test target.
- **Test module pattern**: Using `mod common;` include pattern via path attribute in test files (no separate crate).
- **Test harness scope**: Included in this plan (step-04) to ensure tests can run end-to-end.
