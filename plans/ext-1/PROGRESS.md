# PROGRESS.md — ext-1: Extension Pipeline

## Status
In Progress

## Objective
Add a three-phase extension pipeline to `lcp-server` that lets callers
register request/response transforms at startup. Includes the `SensitiveState`
opaque store with framework-enforced isolation and non-inspection guarantees.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0 | step-01-extensions-module | No | — |
| 1 | step-02-proxy-integration | No | Wave 0 |
| 2 | step-03-tests | No | Wave 1 |

## Dependency Table

| Step | Files | Depends On | Depended By |
|------|-------|------------|-------------|
| step-01 | `crates/lcp-server/src/extensions.rs`, `crates/lcp-server/src/lib.rs` | — | step-02, step-03 |
| step-02 | `crates/lcp-server/src/proxy.rs`, `crates/lcp-server/src/server.rs` | step-01 | step-03 |
| step-03 | `tests/` | step-01, step-02 | — |

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

- [ ] [step-01-extensions-module](./step-01-extensions-module.md) — Define SensitiveState, SensitiveStateBuilder, Extension trait, ExtensionPipeline in new extensions.rs
- [ ] [step-02-proxy-integration](./step-02-proxy-integration.md) — Wire ExtensionPipeline into AppState/ServerConfig and proxy.rs phases
- [ ] [step-03-tests](./step-03-tests.md) — Spec invariant tests for the extension pipeline
