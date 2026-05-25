# PROGRESS.md — tests1

## Status
Queued

## Objective
Add 20+ spec invariant and integration tests covering behavioral correctness (P1), error paths (P2), edge cases (P3), and admin endpoints (P4) using existing TestHarness/MockUpstream infrastructure.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0 | step-01, step-02, step-03 | Yes | — |
| 1 | step-04, step-05, step-06, step-07, step-08, step-09, step-10 | Yes | Wave 0 |
| 2 | step-11, step-12 | Yes | Wave 0 |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| step-01 | `tests/common/mock_upstream.rs` | — | step-11 |
| step-02 | `tests/common/harness.rs` | — | step-12 |
| step-03 | `tests/Cargo.toml` | — | step-10 |
| step-04 | `tests/spec/routing.rs`, `tests/spec/mod.rs` | Wave 0 | — |
| step-05 | `tests/spec/bypass.rs`, `tests/spec/mod.rs` | Wave 0 | — |
| step-06 | `tests/spec/forwarding.rs`, `tests/spec/mod.rs` | Wave 0 | — |
| step-07 | `tests/spec/tracing.rs`, `tests/spec/mod.rs` | Wave 0 | — |
| step-08 | `tests/spec/admin.rs`, `tests/spec/mod.rs` | Wave 0 | — |
| step-09 | `tests/spec/model_extraction.rs` | Wave 0 | — |
| step-10 | `tests/spec/compression.rs`, `tests/spec/mod.rs` | step-03 | — |
| step-11 | `tests/integration/timeout.rs`, `tests/integration/mod.rs` | step-01 | — |
| step-12 | `tests/integration/ttl.rs`, `tests/integration/mod.rs` | step-02 | — |

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

- [ ] [step-01-mock-hang](./step-01-mock-hang.md) — Add `MockResponse::Hang` variant to MockUpstream
- [ ] [step-02-harness-ttl](./step-02-harness-ttl.md) — Add `.ttl()` method to TestHarnessBuilder
- [ ] [step-03-flate2-dep](./step-03-flate2-dep.md) — Add flate2 as dev-dependency in tests/Cargo.toml
- [ ] [step-04-spec-routing](./step-04-spec-routing.md) — Spec tests: unknown provider → 404
- [ ] [step-05-spec-bypass](./step-05-spec-bypass.md) — Spec tests: bypass header, no-cache, trace exclusion
- [ ] [step-06-spec-forwarding](./step-06-spec-forwarding.md) — Spec tests: hop-by-hop header stripping
- [ ] [step-07-spec-tracing](./step-07-spec-tracing.md) — Spec tests: /trace endpoint shape + multi-request edge cases
- [ ] [step-08-spec-admin](./step-08-spec-admin.md) — Spec tests: all admin endpoints (health, stats, cache)
- [ ] [step-09-spec-model-extraction](./step-09-spec-model-extraction.md) — Extend model_extraction with OpenRouter + fallback-None tests
- [ ] [step-10-spec-compression](./step-10-spec-compression.md) — Spec tests: compressed request body decompressed before hashing
- [ ] [step-11-integration-timeout](./step-11-integration-timeout.md) — Integration tests: upstream timeout + unreachable → 502/504
- [ ] [step-12-integration-ttl](./step-12-integration-ttl.md) — Integration tests: TTL expiry and TTL=0 never-expire
