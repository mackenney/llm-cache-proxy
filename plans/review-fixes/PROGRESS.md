# Review Fixes — PROGRESS

Fixes 9 issues (2 blockers, 4 important, 3 suggestions) from code review of the SSE-aware secret-scrubbing feature.

## Wave 0 (parallel — no inter-step deps)

| Step | File(s) | Issues | Status |
|---|---|---|---|
| 01 | `crates/lcp-server/src/ext/sse_unscrub.rs` | B1 (frame reconstruction), I2 (empty-chunk latch), S1 (mem::take) | ⬜ |
| 02 | `crates/lcp-server/src/ext/scrub.rs` | I3 (error_stream on missing key), S2 (hex crate), S3 (module doc) | ⬜ |
| 03 | `crates/lcp-server/src/proxy.rs` | B2 (UTF-8 split across chunks) | ⬜ |
| 04 | `crates/lcp-server/SPEC.md` | I1 (SSE detection wording) | ⬜ |

## Wave 1 (depends on wave 0)

| Step | File(s) | Issues | Status |
|---|---|---|---|
| 05 | `crates/lcp-server/src/ext/sse_unscrub.rs`, `tests/integration/scrub.rs` | B1 test fix, I4 new integration test | ⬜ |

## Acceptance

All steps done when:
- `cargo nextest run` passes (currently 158 tests + new tests from step 05)
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo fmt --check` clean
