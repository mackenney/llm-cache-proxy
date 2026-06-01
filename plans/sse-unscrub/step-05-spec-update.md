# Step 05: Spec Update — Normative SSE-Aware Unscrubbing Text

## Context

### Overall Objective

Implement SSE-aware unscrubbing so that fake keys distributed token-by-token across
Anthropic/OpenAI/Gemini SSE `data:` events are detected at the text level and replaced
before the response reaches the client or cache.

### Phase Context

The implementation is complete (steps 01–03) and the integration tests pass (step 04 in
parallel). This step finalises the specification: the `TODO` preamble in
`crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing` must be removed and replaced with
normative MUST/SHOULD language that describes the implemented behaviour. `MASTER_PROGRESS.md`
must be updated accordingly.

### This Step

Two file edits:
1. `crates/lcp-server/SPEC.md` — remove the `> **TODO:** …` block (4 lines) and replace it
   with a short normative introduction paragraph.
2. `MASTER_PROGRESS.md` — add a Completed entry for this work (the `## Known Gaps` table
   already has no SSE-unscrub row — skip any removal step).

## Prerequisites

- Step 02 merged (implementation exists). This step can run in parallel with step 04.

## Files to Read Before Starting

- `crates/lcp-server/SPEC.md` — read the full `§SSE-Aware Unscrubbing` section (current
  lines ~296–340) carefully before editing
- `MASTER_PROGRESS.md` — understand the Known Gaps table format

## Implementation

### Task 1: Remove TODO block from `SPEC.md`

Locate and **delete** the following four lines (the `>` block immediately after the
`### SSE-Aware Unscrubbing` heading):

```
> **TODO:** This section describes a known gap. The spec text below is a rough
> description of the requirement; it will be polished when the fix is implemented.
> Failing tests exist for the Anthropic case; tests for the remaining providers are
> pending alongside the implementation.
```

(The exact line count may vary slightly; match by content, not line number.)

### Task 2: Insert normative introduction after the heading

After the `### SSE-Aware Unscrubbing` heading (and after the now-deleted TODO block), insert
the following paragraph **before** the `**Problem.**` paragraph:

```markdown
`ScrubExt::on_response_stream` MUST apply unscrubbing at the **semantic SSE text level**
for responses where the first bytes of the stream match the `data: ` SSE prefix.
The raw-byte Aho-Corasick approach (`unscrub_stream`) remains in use for non-SSE responses,
where it works correctly. The two paths are selected automatically; no configuration is
required.
```

### Task 3: Update the "Current state" paragraph

Locate the `**Current state.**` paragraph (currently near the end of the section):

```
**Current state.** `ScrubExt::on_response_stream` passes the raw byte stream directly
to `unscrub_stream` regardless of content type. This is correct for non-SSE responses
and broken for SSE responses. A failing integration test covers the Anthropic case
(`unscrub_restores_secret_from_anthropic_sse_stream`). Tests for OpenAI, OpenRouter,
and Gemini SSE formats are pending alongside the implementation.
```

Replace it with:

```markdown
**Implementation.** `ScrubExt::on_response_stream` wraps the response stream in
`SseUnscrubStream`, which auto-detects the response type by peeking at the first bytes.
For SSE streams it buffers all frames, accumulates provider text fields across events,
runs `unscrub_stream` on the concatenated text, then redistributes the restored text back
into the original frames (all restored text is placed in the first text event; subsequent
text events carry an empty string). For non-SSE streams the raw bytes are passed through
`unscrub_stream` unchanged. Integration tests cover all four providers: Anthropic, OpenAI,
OpenRouter (identical format to OpenAI), and Gemini.
```

### Task 4: Update `MASTER_PROGRESS.md`

> **Pre-Implementation Check:** Before editing `MASTER_PROGRESS.md`, run:
> ```
> grep 'SSE-unscrub' MASTER_PROGRESS.md || echo 'correctly absent'
> ```
> The Known Gaps table has no SSE-unscrub row (it was never added; the work went straight
> to In Progress). **Do not add or remove any Known Gaps row.** Proceed directly to
> adding the Completed entry below.

In `## Completed → ### Implementation`, add a new row (use the merge commit hash of this
plan's final step):

```markdown
| SSE-aware unscrubbing: `SseUnscrubStream` replaces raw-byte unscrub for SSE responses; text accumulated across events, fake restored; all 4 providers covered | <commit-hash> |
```

Replace `<commit-hash>` with the actual hash after `git log --oneline -1` following the
final commit.

## Acceptance Criteria

- [ ] `grep -c 'TODO' crates/lcp-server/SPEC.md` outputs `0`
- [ ] `grep -c 'SSE-unscrub' MASTER_PROGRESS.md` outputs `0` (no Known Gaps row exists — already satisfied before edits)
- [ ] `grep -c 'SSE-aware unscrubbing' MASTER_PROGRESS.md` outputs `1` (row added to Completed)
- [ ] `grep -A3 '### SSE-Aware Unscrubbing' crates/lcp-server/SPEC.md` does NOT contain `TODO`
- [ ] `cargo nextest run` exits 0 (spec change must not break anything)
- [ ] `cargo build` exits 0

## Reviewer Instructions

```bash
cd /home/ignacio/pr/llm-cache-proxy

# No TODO in spec
grep -c 'TODO' crates/lcp-server/SPEC.md

# Section heading + first non-empty line after it (must be normative, not TODO)
grep -A5 '### SSE-Aware Unscrubbing' crates/lcp-server/SPEC.md | head -8

# Known Gaps must not contain SSE-unscrub anymore
grep 'SSE-unscrub' MASTER_PROGRESS.md || echo "correctly absent"

# Completed must have SSE entry
grep 'SSE-aware unscrubbing' MASTER_PROGRESS.md

# Tests still pass
cargo nextest run 2>&1 | grep -E 'FAILED|^test result'
```

Expected: `grep -c 'TODO' crates/lcp-server/SPEC.md` → `0`; section now starts with normative
`MUST` text; Known Gaps has no SSE row; Completed has new row; full suite passes.

## Rollback

```bash
git checkout -- crates/lcp-server/SPEC.md MASTER_PROGRESS.md
```
