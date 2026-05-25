# Progress

## Status
Complete

## Task
Produce implementation plan for P1 behavioral correctness + P2 error path tests.

## Output
Plan written to: `artifacts/plan-tests-p1-p2.md`

## Summary
- 9 implementation steps covering 6 P1 gaps and 2 P2 gaps
- P1 (spec invariants): routing.rs, bypass.rs, forwarding.rs, tracing.rs, model_extraction.rs extension
- P2 (integration): timeout.rs with MockUpstream::Hang extension
- Key decision: Extend MockUpstream with Hang variant for timeout testing
- Key decision: Use shutdown-then-connect trick for unreachable testing
- All steps independently committable

## Files Changed
- artifacts/plan-tests-p1-p2.md (created)

## Notes
- Steps 1, 2, 4, 5, 6, 7, 9 can execute in parallel (no dependencies)
- P2 timeout test requires MockUpstream extension (Step 7) before Step 8

---

# Progress (P3 + P4 Planner)

## Status
Complete

## Task
Produce implementation plan for P3 edge cases + P4 admin endpoint tests.

## Output
Plan written to: `artifacts/plan-tests-p3-p4.md`

## Summary
- 11 implementation steps covering 5 P3 edge cases and 6 P4 admin tests
- P3 (spec invariants): compression.rs, tracing.rs (edge cases only), model_extraction.rs extension
- P4 (spec invariants): admin.rs
- P4 (integration): ttl.rs
- Key decision: Extend TestHarnessBuilder with `.ttl(seconds)` method
- Key decision: Add flate2 dev-dep for gzip compression test helpers
- Key decision: TTL tests in integration tier (require real time passage)

## Files Changed
- artifacts/plan-tests-p3-p4.md (created)

## Notes
- No overlap with P1/P2 planner: P1 covers bypass/routing/tracing-endpoint, P3 covers trace-aggregation edge cases
- Steps 3-9 (spec tests) can execute in parallel after Steps 1-2 (harness/dep changes)
