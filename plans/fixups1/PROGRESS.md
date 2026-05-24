# PROGRESS.md

## Status
Not Started

## Objective
Eliminate race-prone `yield_now()` in tests (replace with deterministic write-drain), extract hardcoded channel capacity into `ServerConfig`, and fix misleading `RecordedRequest.path` naming.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0    | step-01, step-02 | Yes | — |
| 1    | step-03 | No | Wave 0 |
| 2    | step-04 | No | Wave 1 |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| step-01 | `tests/common/mock_upstream.rs`, `tests/common/harness.rs` | — | — |
| step-02 | `crates/lcp-server/src/server.rs`, `crates/lcp-server/src/proxy.rs`, `crates/lcp-server/src/router.rs` | — | step-03 |
| step-03 | `crates/lcp-server/src/proxy.rs`, `crates/lcp-server/src/router.rs`, `crates/lcp-server/src/server.rs`, `tests/common/harness.rs` | step-02 | step-04 |
| step-04 | `tests/spec/cache_miss.rs`, `tests/spec/cache_hit.rs`, `tests/common/harness.rs` | step-03 | — |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel (see wave map)
3. After each step: dispatch reviewer agent (see step file for reviewer instructions)
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps in current wave are complete
6. Blockers: stop and report to user with full context

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Steps

- [ ] [step-01-rename-recorded-request-path](./step-01-rename-recorded-request-path.md) — Rename `RecordedRequest.path` to `uri`, add `.path()` method
- [ ] [step-02-server-config-and-appstate](./step-02-server-config-and-appstate.md) — Add `stream_channel_capacity` to `ServerConfig`, add `JoinSet` to `AppState`, refactor `build_router` signature
- [ ] [step-03-wire-joinset-and-capacity](./step-03-wire-joinset-and-capacity.md) — Use `JoinSet::spawn` in `handle()`, read capacity from config, update `serve()` and test harness
- [ ] [step-04-replace-yield-now](./step-04-replace-yield-now.md) — Replace all `yield_now()` calls with `harness.wait_for_writes()`
