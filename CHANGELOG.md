# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- next-header -->

## [Unreleased] - ReleaseDate

## [0.0.4] - 2026-06-12

### Added

- **`--version` / `-V` flag**: clap now exposes the binary version; the
  version is also logged at `INFO` level on startup.
- **Cassette integration test tier** (`tests/integration/`): 34 recorded
  fixtures spanning all four providers (Anthropic, OpenAI, OpenRouter,
  Gemini) exercise cache hit/miss, SSE passthrough, error responses,
  concurrent requests, and SSE detection. Tests run fully offline against a
  mock upstream — no live API key required.

### Fixed

- **`flush_safe_prefix` panic on multibyte characters** (`SseRestoreStream`): when
  `nominal_target` (the safe-flush boundary derived from fake key lengths) landed
  inside a multibyte codepoint — 4-byte emoji, 3-byte CJK, or 2-byte Greek — the
  `accum[..target]` slice panicked with "end byte index N is not a char boundary".
  The target is now walked back with `is_char_boundary` before any slice.
  Three regression tests cover each codepoint width.

- **SSE terminal-event ordering** (`SseRestoreStream`): terminal frames
  (`content_block_stop`, `message_stop`, `[DONE]`, `response.output_item.done`,
  etc.) were forwarded to the client before the held accumulator was flushed,
  truncating doppel-restored content (e.g. tool-call arguments) in the final
  chunk. Block-scope flush now fires on Anthropic `content_block_stop`;
  stream-scope flush fires on all other terminal events.

- **`flush_safe_prefix` with embedded fake keys**: when a fake key was
  embedded inside JSON (e.g. `partial_json={"key":"FAKE_KEY"}`), the
  surrounding JSON bytes pushed the accumulator past `max_fake_len`. The
  sliding window emitted the first few bytes of the fake as a separate
  fragment, preventing Aho-Corasick from matching the complete fake — the
  original secret was never restored. The flush boundary now retracts to
  just before any partial-fake prefix found at the tail of the safe region.

- **Responses API synthetic frame event names**: `ResponsesApiDelta` and
  `ResponsesApiDone` synthetic frames were emitting abbreviated event types
  (`output_text`) instead of the full standard names
  (`response.output_text.delta` / `response.output_text.done`), breaking
  clients that match on the standard event type strings.

- **OpenRouter colocated `finish_reason`**: OpenRouter colocates
  `{"content":""}` with a non-null `finish_reason` in the final Chat
  Completions chunk. `extract_fields` treated the empty string as
  extractable content, preventing `classify_terminal` from firing the
  terminal flush. Held content was silently dropped. Empty-string content
  is now skipped so the terminal flush fires correctly.

- **Gemini `deferred_passthrough` drain**: Gemini SSE chunks that are
  deferred for passthrough were not drained before the stream-scope terminal
  flush, causing those chunks to be dropped on streams that end with a
  usage-metadata-only frame.

## [0.0.3] - 2026-06-09

### Changed

- **`DoppelExt` now pre-builds the Aho-Corasick automaton once at construction**
  and shares it across requests via `Arc<Detector>`, instead of reconstructing
  it on every Phase 1 call.

- **SSE restore no longer buffers the full response.** Phase 3 now operates a
  per-field sliding window: restored text is emitted to the client as each SSE
  frame arrives, with at most `max_fake_len` bytes held in reserve to handle
  fakes that straddle frame boundaries. Initial TTFB latency is bounded by
  `max_fake_len / text_generation_rate` (typically 500 ms – 4 s for standard
  secret classes); after the initial hold, restored frames flow in real time.
  The full-response-buffer approach has been removed.

## [0.0.2] - 2026-06-06

### Added

- `--body-limit` / `LCP_BODY_LIMIT` flag to configure the maximum incoming
  request body size (default 100 MiB; 0 = no limit). Requests exceeding the
  limit are rejected with HTTP 413 before reaching the upstream.

### Changed

- Default incoming body size limit raised from axum's implicit 2 MiB to
  100 MiB.

### Fixed

- `--config` CLI flag now correctly takes precedence over the `LCP_CONFIG`
  environment variable. Previously the env var could win over an explicit flag.

## [0.0.1] - 2026-06-06

Initial release.

### Added

- HTTP proxy for LLM API calls supporting four providers: Anthropic, OpenAI,
  OpenRouter, and Gemini. Clients point their SDK base URL at lcp
  (`http://127.0.0.1:9001/<provider>`) with no other changes required.
- BLAKE3-based deterministic cache key derived from provider, model, and
  normalized request body. API key rotation never busts the cache; headers
  are excluded from the key.
- Provider-aware request normalization: non-deterministic fields are stripped
  per-provider before hashing.
- SQLite-backed response cache (`~/.cache/lcp/cache.db`) with configurable TTL
  (`--ttl` / `LCP_TTL`; default 0 = never expire). Expired entries are treated
  as misses on access (lazy expiry, not proactive deletion).
- True streaming on cache miss: response chunks are forwarded to the client
  as they arrive from upstream without buffering the full body.
- Full-speed replay on cache hit: stored SSE chunks are replayed sequentially,
  preserving original frame boundaries. No artificial inter-chunk delay.
- Response headers on every proxied request: `x-lcp-cache: HIT|MISS|BYPASS`
  and `x-lcp-key: <12-char key prefix>`.
- Cache bypass via `x-lcp-bypass: 1` request header: skips both cache read
  and cache write.
- Request decompression: `Accept-Encoding` is stripped before forwarding so
  upstream responses arrive uncompressed; compressed downstream request bodies
  are decompressed before hashing and forwarding.
- Configurable upstream request timeout (`--timeout` / `LCP_TIMEOUT`;
  default 300 s; 0 = no timeout).
- Secret swap/restore extension via `doppel`: real API keys in request bodies
  and SSE response streams are replaced with fakes before caching and restored
  on replay. SSE-aware restore operates at the semantic text level so secrets
  split across multiple `data:` frames are handled correctly across all four
  providers. Enabled via `[extensions.doppel]` in the config file.
- Extension pipeline with Phase 1/2/3 hooks and an opaque sensitive state
  store that persists data across hook phases without exposing it to cache
  storage.
- Tracing: attach `x-lcp-trace: <id>` to requests to group them into a named
  session. The `(trace_id, cache_key)` pair is persisted for every cached
  non-bypass request.
- Cache inspection endpoint: `GET /cache/<key>` returns the full exchange
  (request body, response chunks with `offset_ms` timestamps, metadata).
- Trace query endpoints: `GET /trace/<id>` (metadata) and
  `GET /trace/<id>?full=true` (full exchange per entry).
- Stats and admin endpoints: `GET /stats` (hit/miss counts, bytes served,
  entry count, per-model breakdown), `DELETE /stats` (reset counters),
  `DELETE /cache` (purge all entries and traces), `GET /` (health check).
- TOML config file at `$XDG_CONFIG_HOME/lcp/config.toml`; override with
  `--config` / `LCP_CONFIG`. CLI flags beat env vars beat config file.
- `--print-config` flag to dump the effective resolved configuration as TOML
  and exit.
- Per-provider upstream URL overrides via `--anthropic-upstream`,
  `--openai-upstream`, `--openrouter-upstream`, `--gemini-upstream` and
  corresponding `LCP_*_UPSTREAM` env vars.
- Gemini path-based model extraction: model name is parsed from the URL path
  (e.g. `gemini-2.5-flash` from `/models/gemini-2.5-flash:generateContent`).

<!-- next-url -->
[Unreleased]: https://github.com/mackenney/llm-cache-proxy/compare/v0.0.4...HEAD
[0.0.4]: https://github.com/mackenney/llm-cache-proxy/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/mackenney/llm-cache-proxy/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/mackenney/llm-cache-proxy/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/mackenney/llm-cache-proxy/tree/v0.0.1
