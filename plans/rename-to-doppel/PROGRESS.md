# PROGRESS.md

## Status
In Progress

## Objective
Rename the `its-classified` library to `doppel` and update all consumers in
`lcp`. No behavior changes — pure rename/restructure.

## Open Decisions

1. **`from_patterns_file` method rename:** `DoppelExt::from_patterns_file` →
   `DoppelExt::from_secrets_file`? Both planners flagged this. The library
   method name should match the config key. Assuming YES — rename to
   `from_secrets_file`. If user disagrees, revert just this symbol.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0 | step-01 | No | — |
| 1 | step-02 | No | Wave 0 |
| 2 | step-03, step-04 | Yes | step-02 |
| 3 | step-05, step-06 | Yes | Wave 2 |

## Dependency Table

| Step | Repo | Key Files | Depends On | Depended By |
|------|------|-----------|------------|-------------|
| step-01 | its-classified | all src/, cli/, tests/, Cargo.toml, SPEC.md, AGENTS.md | — | step-02 |
| step-02 | lcp | lcp-server ext, workspace Cargo.toml | step-01 | step-03, step-04 |
| step-03 | lcp | lcp/src/main.rs | step-02 | step-05 |
| step-04 | lcp | tests/integration/scrub.rs, tests/Cargo.toml | step-02 | step-05 |
| step-05 | lcp | SPEC.md, crates/lcp-server/SPEC.md, AGENTS.md | step-03, step-04 | step-06 |
| step-06 | lcp | Cargo.toml (remove [patch]) | step-05, step-01 pushed | — |

## Orchestrator Protocol
1. Read this file to identify current wave
2. **Pre-flight:** verify both repos clean, create feature branches, add `[patch]` to lcp `Cargo.toml`
3. Dispatch all steps in current wave (see wave map)
4. After each step: dispatch reviewer agent (see step file for reviewer instructions)
5. Mark step complete only after reviewer passes
6. Advance to next wave only when all steps in current wave are complete
7. **Before step-06:** push step-01 branch to sourcehut remote
8. Blockers: stop and report to user with full context

## Pre-Flight (orchestrator action, not a worker step)

Before dispatching any workers:
1. Verify both repos are on `main` with clean working trees
2. Create a feature branch in each repo:
   - its-classified: `ignacio@doppel/rename-crate`
   - lcp: `ignacio@doppel/update-consumer`
3. Add local path override to lcp `Cargo.toml`:
   ```toml
   [patch."https://git.sr.ht/~mackenney/its-classified"]
   doppel = { path = "/home/ignacio/pr/its-classified" }
   ```
   This lets lcp compile against the local renamed crate without pushing.

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Steps

- [ ] [step-01-rename-library](./step-01-rename-library.md) — rename its-classified to doppel: crate identity, all API symbols, CLI, tests, docs
- [ ] [step-02-rename-lcp-server-ext](./step-02-rename-lcp-server-ext.md) — rename lcp-server ext layer, Cargo deps, file names
- [ ] [step-03-rename-lcp-cli](./step-03-rename-lcp-cli.md) — rename lcp CLI config struct, TOML keys, log messages (with backward compat)
- [ ] [step-04-rename-tests](./step-04-rename-tests.md) — rename integration test file, imports, macros, function names
- [ ] [step-05-update-docs](./step-05-update-docs.md) — update SPEC.md files, AGENTS.md, inline doc comments
- [ ] [step-06-finalize-dependency](./step-06-finalize-dependency.md) — remove [patch], switch to remote git dep, clean up MASTER_PROGRESS
