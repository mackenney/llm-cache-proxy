# PROGRESS.md

## Status
In Progress

## Objective
Add configurable `--body-limit` / `LCP_BODY_LIMIT` flag (default 100 MiB, 0 = no limit) to lcp, applying `DefaultBodyLimit` layer on proxy routes only.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0    | step-01 | No | — |
| 1    | step-02, step-03 | Yes | Wave 0 |
| 2    | step-04, step-06 | Yes | Wave 1 |
| 3    | step-05 | No | Wave 2 (step-04) |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| step-01 | `crates/lcp-server/src/server.rs` | — | step-02, step-03, step-04 |
| step-02 | `crates/lcp-server/src/router.rs` | step-01 (ServerConfig shape) | step-05 |
| step-03 | `crates/lcp/src/main.rs` | step-01 (ServerConfig shape) | step-04, step-05, step-06 |
| step-04 | `tests/common/harness.rs` | step-01, step-03 | step-05 |
| step-05 | `tests/integration/body_limit.rs`, `tests/integration/mod.rs` | step-02, step-03, step-04 | — |
| step-06 | `tests/e2e/cli.rs` | step-03 | — |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps complete
6. Blockers: stop and report to user

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit with message "step-NN: <name>"
- Workers: Report: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim.

## Steps
- [ ] [step-01-server-config](./step-01-server-config.md) — Add `body_limit_bytes` field to `ServerConfig`
- [ ] [step-02-router-layer](./step-02-router-layer.md) — Apply `DefaultBodyLimit` layer to proxy routes
- [ ] [step-03-cli-flag](./step-03-cli-flag.md) — Add `--body-limit` CLI flag, env var, config file, and print-config
- [ ] [step-04-harness](./step-04-harness.md) — Add `body_limit_bytes` field and builder method to test harness
- [ ] [step-05-integration-tests](./step-05-integration-tests.md) — Add `tests/integration/body_limit.rs` with 413 tests
- [ ] [step-06-e2e-cli-test](./step-06-e2e-cli-test.md) — Add e2e test for `--print-config` default body_limit
