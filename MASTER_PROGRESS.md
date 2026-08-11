# Master Progress

Read this file to understand the current state of lcp. One-liner per item; all detail is in plan files and git history.

---

## In Progress

_(none)_

## Queued

### System-prompt cache-key normalization

Exclude volatile lines injected by agentic harnesses (e.g. `Current date: …`, `Current working directory: …`) from the cache key so the same logical prompt gets the same key across days and directories. The forwarded request is never modified — only the bytes fed into BLAKE3 are affected. Configured via `[extensions.system_prompt_normalize]` as a list of regexes matched against the system message body.
## Completed

### Foundation
- Cargo workspace, crate stubs, rustfmt/clippy config
- Root `SPEC.md`: full behavioral contract (proxy, cache key, providers, SSE, tracing, stats, config)
- `crates/lcp-core/SPEC.md` and `crates/lcp-server/SPEC.md`: per-crate behavioral contracts
- `AGENTS.md`, `MASTER_PROGRESS.md`: project constitution and governance
- `lcp-core` skeleton: types, provider enum, BLAKE3 hash, SQLite cache
- `lcp-server` skeleton: axum router, proxy handler, stats endpoints
- `lcp` CLI skeleton: clap binary

### Implementation
- Rename `its-classified` → `doppel`, `swap`/`restore` verbs, `SecretsFile`, `DoppelExt` throughout — 887d985
- Fix stale scrub/unscrub vocabulary in docs, tests, CLI strings, SPEC.md — 253ffa1
- Fix spec gaps: tracing, bypass headers, decompression, timeout, stats (17 tests) — 6dfbc71
- Gemini provider, `GET /cache/<key>`, `GET /trace/<id>?full=true`, `FullEntry`/`inspect`/`inspect_trace` — c0230a4
- `MockUpstream` + `TestHarness` + 10 Priority 1 spec invariant tests (cache hit/miss) — eda6d31
- Replace full-body buffering with true streaming: spawn+channel for miss, `stream::iter` for hit — 3ddc80e
- Fixups: rename `RecordedRequest.path→uri`, extract channel capacity, replace `yield_now` with deterministic drain — 83e2a19
- Gemini path-based model extraction: `extract_model_from_path` + proxy wiring + 5 spec invariant tests — fff8d01
- 32 new tests (routing, bypass, forwarding, tracing, admin, model extraction, compression, timeout, TTL); fix expired-entry miss counter gap — eab47cf
- Provider-aware cache key normalization: `normalization_strip_fields`, `cache_key(provider,…)`, 9 spec invariant tests — 4b01572
- Extension pipeline: Phase 1/2/3 hooks, `SensitiveState` opaque store, proxy wiring, 10 spec invariant tests — be99fd3
- Code review fixes: 3 BLOCKERs, 3 IMPORTANTs, 3 perf, 3 maintenance — f93fd81
- SSE-aware restoring: `SseRestoreStream` replaces raw-byte restore; all 4 providers covered — 6d30152
- Fix SSE detection for Anthropic real API: `is_sse_first_chunk` detects `event: ` prefix; E2E verified — 6e39cc0
- Code review fixes: frame reconstruction bug, UTF-8 chunk corruption, empty-chunk latch, partial-state error, hex crate, SPEC update (159 tests) — a834725
- SSE multi-buffer field coverage: FieldKey/ExtractedField architecture, VC-SSE-1..13 (all providers), 3-round code review, E2E verified vs real APIs — cdf8fc5
- Body size limit: `--body-limit` / `LCP_BODY_LIMIT` (default 100 MiB, 0 = no limit), `DefaultBodyLimit` on proxy routes, 2 spec invariants + 3 integration tests — 47747de
- Sliding-window SSE restore: `SseRestoreStream` emits incrementally per-FieldKey (max_fake_len hold), synthetic frames, spec invariant tests — e71a2b5
- SSE terminal-event ordering fix: block+stream-scope flush before terminal frames; `classify_terminal`, `flush_accumulators_where`; 7 spec invariants + 1 integration regression test (VC-SSE-14..20) — aece11c
- 5-round code review + E2E fixes: `is_sse_first_chunk` spaceless detection, Gemini deferred drain before terminal, `flush_safe_prefix` split-fake bug, synthetic Responses API event type names, empty-content finish_reason routing; 253 tests, all providers verified — 0bc6827
- Cassette-based integration tier: 34 TOML fixtures (4 providers), `MockResponse::Recorded`, wire-format tests, 3 regression guards (`flush_safe_prefix`, `response.output_text.delta`, `content:"" + finish_reason`), error/concurrent/SSE-detection coverage; 198 tests total — 1377970
- Bump `doppel` to 0.1.0: AWS AKIA/ASIA default trailing-run-guard fix, found via production image-upload corruption through the doppel secret-swap extension — 422c99f

---

## Known Gaps

- **OpenAI Responses API `response.completed` event leaks fake** — the final `response.completed` event body contains the fake key because `extract_fields` does not extract from non-delta/done event types; `e2e_openai_responses_api` intentionally omits `assert_absent` for this reason. Low-priority: the secret is already restored in all `delta`/`done` events.
- **Responses API `response.reasoning_summary_text.done` unhandled** — SPEC only mandates `.delta` for reasoning summary; if OpenAI emits a `.done` event with the full reasoning text containing a fake, it leaks. Low-priority: no MUST requirement covers this event type.
- **Gemini `GeminiText` same-thought multi-event accumulation** — `GeminiText { thought: bool }` accumulates all same-thought text parts into one buffer; if two separate streaming events each deliver a non-thought text part, the second frame's `parts[N].text` is written as empty string after restoration. Current Gemini streaming never does this. Outside VC-SSE-9 scope.
