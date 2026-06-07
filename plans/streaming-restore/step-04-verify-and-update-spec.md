# Step 4: Final Verification + SPEC Update + MASTER_PROGRESS

## Objective

Run full test suite, update lcp-server SPEC.md to document the sliding-window implementation as the current behavior (removing any "buffering" language), and update MASTER_PROGRESS.md.

## Prerequisite

Steps 0-3 complete. All tests pass.

## Part A: Full Test Suite

Run every test tier and confirm green:

```bash
# All spec invariant tests
cargo nextest run --test spec

# All integration tests (includes VC-SSE-1..13 + new sliding-window edge cases)
cargo nextest run --test integration

# All inline unit tests
cargo nextest run -p lcp-server
cargo nextest run -p lcp-core

# Full workspace
cargo nextest run

# Clippy
cargo clippy --workspace --all-targets -- -D warnings

# Format
cargo fmt --check
```

If E2E tests are available and the environment supports them:
```bash
cargo nextest run --test e2e --features test-e2e
```

## Part B: Update lcp-server SPEC.md §SSE-Aware Restore

The current `crates/lcp-server/SPEC.md` §SSE-Aware Restore (line ~300) describes the semantic restore procedure but does not mention the sliding window. The root `SPEC.md` already has §SSE-Aware Restore: Sliding Window. Ensure the lcp-server SPEC aligns.

**Changes to `crates/lcp-server/SPEC.md`:**

1. In the SSE-Aware Restore section, add a subsection or paragraph stating:
   - Phase 3 SSE restore uses a per-field sliding window (referencing root SPEC.md §SSE-Aware Restore: Sliding Window).
   - `SseRestoreStream` emits restored frames incrementally, not after full buffering.
   - The hold window is `max_fake_len` bytes per FieldKey.
   - Outbound frames are synthetic with provider-specific JSON structure.
   - Non-SSE path is unchanged (byte-level restore).

2. Remove or update any language implying full buffering is required.

**Do NOT duplicate the full sliding-window spec from root SPEC.md.** Reference it and describe the implementation-level details specific to lcp-server.

## Part C: Update MASTER_PROGRESS.md

1. Remove the streaming-restore entry from Queued (or In Progress).
2. Add to Completed: `- Sliding-window SSE restore: SseRestoreStream emits incrementally per-FieldKey, spec invariant tests — <commit-hash>`
3. Delete the `plans/streaming-restore/` directory (per project convention: plans deleted on completion).

## Part D: Commit

Stage all changes and commit:
```bash
git add -A
git commit -m "Implement sliding-window SSE restore with spec invariant tests"
```

Use the actual commit hash in MASTER_PROGRESS.md (amend if needed after commit).

## Exit Criteria

- All test tiers pass.
- lcp-server SPEC.md updated to reflect sliding-window behavior.
- MASTER_PROGRESS.md updated with completion entry.
- `plans/streaming-restore/` deleted.
- Clean commit on the branch.
