# Step 04 — SPEC.md SSE detection wording update (I1)

**File:** `crates/lcp-server/SPEC.md`
**Wave:** 0 (no deps)

---

## I1 — SSE detection wording omits `event: ` prefix

### Current text (line 299-300):

```markdown
`ScrubExt::on_response_stream` MUST apply unscrubbing at the **semantic SSE text level**
for responses where the first bytes of the stream match the `data: ` SSE prefix.
```

### New text:

```markdown
`ScrubExt::on_response_stream` MUST apply unscrubbing at the **semantic SSE text level**
for responses where the first bytes of the stream match the `data: ` or `event: ` SSE
prefix. Anthropic's real API starts each event with a named `event:` line (e.g.,
`event: message_start`) before the `data:` line, so the first bytes of the stream are
`event: ` rather than `data: `.
```

---

## Acceptance

Read the file and confirm the updated wording. No code changes, no test impact.
