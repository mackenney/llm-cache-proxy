# lcp-server Specification

> The key words MUST, MUST NOT, SHOULD, SHOULD NOT, MAY are used per RFC 2119.

## Purpose

`lcp-server` is the HTTP proxy layer. It binds a TCP listener, routes
incoming requests to the correct upstream provider, applies cache
read/write/bypass logic, streams responses, and serves admin endpoints.
All domain logic (key computation, storage) is delegated to `lcp-core`.

## Non-Goals

- Cache key computation or storage — owned by `lcp-core`.
- CLI argument parsing — owned by `lcp`.
- TLS termination; the server speaks plain HTTP only.
- Authentication or multi-user isolation.

## Boundary with lcp-core

`lcp-server` receives a fully-constructed `Cache` handle and calls only the
public API defined in `lcp-core`. It MUST NOT open or query the underlying
store directly.

## Configuration

`ServerConfig` holds everything the server needs at startup:

- Bind address (host + port).
- A `Cache` handle (constructed by the caller with the desired TTL and DB
  path).
- Optional upstream URL overrides, one per provider. When absent, the
  provider's default upstream is used.
- Upstream request timeout in seconds (default 300). Applied to the HTTP
  client used for forwarding; a value of 0 means no timeout.
- Maximum incoming request body size in bytes (default 104_857_600 — 100 MiB).
  A value of 0 means no limit. Requests exceeding the limit MUST be rejected
  with HTTP 413 before reaching the proxy handler.

`ServerConfig` MUST be `Clone + Send + Sync` so it can be shared across
async request handlers.

## Routing

| Method | Path | Handler |
|---|---|---|
| `GET` | `/` | Health check |
| `GET` | `/stats` | Aggregate stats |
| `DELETE` | `/stats` | Reset stats counters |
| `DELETE` | `/cache` | Purge all cache entries |
| `GET` | `/cache/<key>` | Fetch full exchange by cache key. `<key>` MUST be the full BLAKE3 hex digest — the 12-char `x-lcp-key` response header is an observability prefix only and is not accepted here. Returns 404 if not found. |
| `GET` | `/trace/<trace-id>` | Trace lookup (metadata) |
| `GET` | `/trace/<trace-id>?full=true` | Trace lookup with full request/response data |
| `POST`, `GET` | `/<provider>/<*path>` | Proxy handler |

An unrecognised `<provider>` prefix MUST return HTTP 404.

## Proxy Behavior

### Common pre-processing (all proxy requests)

1. Parse `<provider>` from the path prefix via `Provider::from_prefix`; 404
   on unknown.
2. Check for `x-lcp-bypass: 1` header.
3. Compute the cache key by passing the resolved provider, method, path, and body
   to the key computation function.
4. If bypassing: skip to [Bypass](#bypass).
5. Otherwise: attempt a cache read.

### Cache hit

1. Replay the stored chunks to the client sequentially at full speed; do not
   introduce inter-chunk delays.
2. Preserve original chunk boundaries.
3. Include response headers: `x-lcp-cache: HIT`,
   `x-lcp-key: <first-12-chars-of-key>`.

### Cache miss

1. Forward the request to the upstream, preserving the original method, path,
   and query string.
2. Strip these headers before forwarding: `host`, `connection`,
   `transfer-encoding`, `accept-encoding`, `content-encoding`, `content-length`.
3. Stream each response chunk back to the client as it arrives; do not buffer
   the full response before forwarding.
4. If the upstream status is `2xx`, store the exchange via `Cache::put` with
   the provider, extracted `model`, and all received chunks.
5. Include response headers: `x-lcp-cache: MISS`,
   `x-lcp-key: <first-12-chars-of-key>`. These headers are added for
   **all** upstream responses, including non-`2xx`.
6. Non-`2xx` responses MUST NOT be stored. They are forwarded to the
   client with the `x-lcp-cache: MISS` and `x-lcp-key` headers above,
   and the `misses` counter is incremented.

### Model Extraction

The proxy MUST extract a model identifier from each cacheable request and pass
it to `Cache::put`. Extraction rules are provider-specific:

| Provider | Extraction Source |
|---|---|
| Anthropic | `model` field from JSON request body |
| OpenAI | `model` field from JSON request body |
| OpenRouter | `model` field from JSON request body |
| Gemini | Path segment: `/models/{model}:{verb}` pattern |

For Gemini, the model MUST be extracted from the URL path using the pattern
`/models/{model}:{verb}` where `{verb}` is one of `generateContent`,
`streamGenerateContent`, `countTokens`, `embedContent`, or similar.
The model is the segment between `/models/` and the colon-verb suffix
(e.g., `/models/gemini-2.5-flash:generateContent` yields `gemini-2.5-flash`).

If extraction fails for any provider (malformed body, unrecognized path
pattern), the proxy SHOULD store `None` as the model. The cache entry remains
valid; only the model metadata is absent.

### Bypass

1. Forward the request without touching the cache (no read, no write).
2. Return the upstream response as-is.
3. Include response header: `x-lcp-cache: BYPASS`.
4. MUST NOT include an `x-lcp-key` header.
5. MUST NOT record a trace entry.

## Compression

The HTTP client used for upstream requests MUST disable all
content-encoding negotiation (`gzip`, `deflate`, `brotli`, `zstd`) so that
upstream responses — including SSE streams — arrive uncompressed.

Incoming request bodies that are compressed MUST be decompressed before
hashing and forwarding.

## SSE / Streaming

The proxy MUST NOT buffer a complete streaming response before forwarding.
Chunks MUST be written to the downstream client as they arrive from upstream
(cache miss) or as they are read from storage (cache hit). Phase 3 extension
hooks (§Extension Pipeline) MAY buffer the response for semantic transforms;
this is an accepted trade-off documented in each extension's spec section.

`Accept-Encoding` MUST be stripped from all forwarded requests to prevent
upstream providers from compressing SSE streams.

## Tracing

When an incoming proxy request carries an `x-lcp-trace: <trace-id>` header
and the request is not a bypass:

- After a successful cache write (miss) or cache read (hit), the server MUST
  call `Cache::record_trace(trace_id, cache_key)`.

### `GET /trace/<trace-id>`

Accepts an optional `?full=true` query parameter.

Without `?full=true`: returns metadata for all cache entries in the trace, ordered by `created_at`.

```json
{
  "trace_id": "<trace-id>",
  "entries": [
    { "key": "...", "created_at": "...", "status": 200, "hit_count": 3 }
  ]
}
```

With `?full=true`: each entry also includes `provider`, `model`, `content_type`,
`req_bytes`, `resp_bytes`, `request: {method, path, body}`, and `chunks: [{data, offset_ms}]`.

An unknown trace ID returns the same shape with an empty `entries` array.

## Admin Endpoints

### `GET /`

Returns `{"status": "ok"}`. Used as a health probe.

### `GET /stats`

Returns aggregate counters and entry metadata:

```json
{
  "hits": 312,
  "misses": 87,
  "bytes_served_from_cache": 4182404,
  "entries": 87,
  "by_model": { "anthropic/claude-sonnet-4": 46, "gpt-4o": 41 }
}
```

### `DELETE /stats`

Resets stats counters to zero. Per-entry `hit_count` values and cache entries
are unaffected. Returns `{"cleared": true}`.

### `DELETE /cache`

Deletes all cache entries and trace entries. Stats counters are unaffected.
Returns `{"cleared_entries": <n>}`.

## Extension Pipeline

The server supports an ordered, opt-in pipeline of extensions that may
transform requests and responses as they pass through the proxy. Extensions
are registered at startup and invoked on every proxied request. Admin
endpoints (`/stats`, `/cache`, `/trace/…`) MUST NOT invoke any extension hook.

### ProxyCtx

`ProxyCtx` is a value passed to extension hooks that describes the in-flight
request. It carries:
- The resolved `Provider`.
- The HTTP method.
- The request path (after the provider prefix is stripped).
- The cache key — present in Phase 2 and Phase 3 hooks; absent in Phase 1.

`ProxyCtx` MUST be `Clone` so the pipeline runner can give each extension
its own copy without sharing references.

### SensitiveState

`SensitiveState` is an immutable string-to-string map. An extension produces
it in Phase 2 via `SensitiveStateBuilder` and receives it back in Phase 3.
It is the only mechanism for carrying per-request extension state across the
Phase 2 → Phase 3 boundary.

**Framework guarantees — the pipeline runner MUST:**

- MUST NOT inspect, iterate, log, trace, or print `SensitiveState` contents.
  The `Debug` implementation MUST render as `SensitiveState { <redacted> }`.
  No other representation is provided.
- MUST NOT expose the `SensitiveState` produced by extension `i` to any
  other extension, at any point.
- MUST NOT persist `SensitiveState` to any storage medium.
- MUST drop each `SensitiveState` no later than when the response stream for
  the request that produced it is fully consumed or terminates with an error.
- MUST guarantee the only code path that can call `get` on an extension's
  `SensitiveState` is the Phase 3 hook of the same extension instance that
  produced it.

`SensitiveStateBuilder` is the sole construction path. Extensions call
`set(key, value)` during Phase 2 and `build()` to seal it. After `build()`
no mutation is possible.

`SensitiveState` exposes a single read operation: `get(key) -> Option<&str>`.
It MUST NOT provide iteration, serialization helpers, or any method that
could assist bulk extraction of its contents.

### Extension Protocol

An extension is a value implementing the `Extension` contract.
Implementors MUST be `Send + Sync + 'static`.

The pipeline defines three hook points:

| Hook | Phase | Fires | Input | Output |
|---|---|---|---|---|
| `on_request_body` | 1 | Before cache key computation | owned `ProxyCtx`, `Bytes` | `Bytes` |
| `on_upstream_body` | 2 | After cache lookup, on miss only | owned `ProxyCtx`, `Bytes` | `Bytes` + `SensitiveStateBuilder` |
| `on_response_stream` | 3 | After upstream responds, on miss only | owned `ProxyCtx`, `SensitiveState`, stream | stream |

Default behavior for every hook is the identity transform. An extension that
does not need a hook MUST still satisfy the contract but MAY return inputs
unchanged.

**Pipeline invariants:**

- Phase 1 hooks MUST fire for every proxied request, including bypass and
  cache-hit requests.
- Phase 2 and Phase 3 hooks MUST NOT fire on cache hits or bypass requests.
- The pipeline MUST run hooks in registration order.
- Each Phase 2 hook produces a `SensitiveState` that is passed exclusively to
  the Phase 3 hook of the same extension in the same request; states MUST NOT
  be reordered or mixed across extensions.
- If any Phase 2 hook returns an error, the pipeline MUST fail closed: the
  upstream request MUST NOT be sent, and the client MUST receive a 5xx
  response. Phase 3 hooks for that request MUST NOT fire.
- The cache write MUST use the byte sequence produced by the Phase 3
  transformed stream, not the raw upstream bytes.

### Pipeline and Cache Interaction

Phase 1 fires before the cache key is computed. Transforms applied in Phase 1
affect the cache key and the stored request. Use Phase 1 for normalization
transforms where logically equivalent requests should share a cache entry.

Phase 2 fires after the cache key is computed, on cache misses only. The
cache key is derived from the Phase-1-transformed body, not the
Phase-2-transformed body. Use Phase 2 for wire-only transforms where the
cached content should reflect the post-Phase-3 (restored) value, not the
value sent over the wire.

Phase 3 fires after the upstream responds. Its output is what the client
receives and what is written to the cache. For a swap/restore extension pair,
Phase 3 restores the original bytes, so the cache stores originals while the
wire carried only fakes.

### SSE-Aware Restore

Phase 3 MUST apply restoring at the **semantic SSE text level** for
responses where the first bytes of the stream match the `data: ` or `event: `
SSE prefix. Anthropic streams begin with a named `event:` line (e.g.,
`event: message_start`) before the `data:` line, so the first bytes are
`event: ` rather than `data: `. The non-SSE (plain JSON) response path MUST
continue to use byte-level restore unchanged — it works correctly there.

**Problem.** Byte-level restore works for non-SSE responses where the full
fake appears as a contiguous byte sequence. In SSE streaming responses, text
content is delivered token-by-token inside individual `data:` events. Each
event carries only a fragment of the fake, separated by SSE framing bytes.
The full fake never appears as a contiguous byte sequence in the raw stream,
so byte-level matching fails silently — passing fakes to the client and
writing them to the cache.

**Semantic restore procedure:**

1. Parse each `data:` line as a JSON object.
2. Locate all applicable provider-specific content fields (see tables below).
3. Accumulate text from each content field independently across consecutive
   events; a fake may begin in one event and end in a later one.
4. When a complete fake is detected in the accumulated text, replace it with
   the decrypted original.
5. Re-encode the corrected text back into the SSE event and emit it.

**Implementation — sliding window.** Phase 3 MUST NOT buffer the entire response
before emitting output. `SseRestoreStream` implements the sliding-window algorithm
specified in root `SPEC.md §SSE-Aware Restore: Sliding Window`: each FieldKey
maintains an accumulation buffer; bytes beyond the `max_fake_len` hold window are
safe to restore and emit immediately as synthetic frames with provider-specific
JSON structure. At stream EOF the remaining hold buffer is flushed. Output flows
in real time — TTFB latency is bounded by `max_fake_len / text_generation_rate`.
Outbound frame granularity MAY differ from the original.

Terminal events (provider-specific stop signals) MUST also trigger an early
complete flush of relevant buffers before the terminal frame is forwarded; see
§Terminal Event Ordering below.

#### Provider Content Fields

Every field listed as MUST below MUST have its text extracted, accumulated,
and restored. Fields listed as MUST NOT MUST be passed through byte-for-byte
unmodified — even if their content coincidentally matches a fake pattern.

##### Anthropic

All content fields below appear within events where `type == "content_block_delta"`.

| `delta.type` | Content field | Requirement |
|---|---|---|
| `text_delta` | `delta.text` | MUST restore |
| `thinking_delta` | `delta.thinking` | MUST restore |
| `input_json_delta` | `delta.partial_json` | MUST restore |
| `signature_delta` | `delta.signature` | MUST NOT modify |

A single Anthropic response MAY interleave blocks of different delta types
(e.g., a sequence of `thinking_delta` events followed by `text_delta` events).
Each content field MUST be accumulated independently, keyed by the combination
of `delta.type` and the `index` field that identifies the content block.

##### OpenAI / OpenRouter — Chat Completions

These fields appear in streaming chat completion delta events.

| Content field | Requirement |
|---|---|
| `choices[0].delta.content` | MUST restore |
| `choices[0].delta.tool_calls[N].function.arguments` | MUST restore |
| `choices[0].delta.reasoning_content` | MUST restore |
| `choices[0].delta.function_call.arguments` | MUST restore |
| `choices[0].delta.refusal` | MUST restore |

OpenRouter uses the OpenAI-compatible chat completion format. The same field
extraction rules apply under the OpenRouter provider.

`tool_calls` is an array; each element carries an `index` field identifying
which tool call it belongs to. Text MUST be accumulated independently per
tool-call index.

##### OpenAI — Responses API (`v1/responses`)

The Responses API uses a wholly different event schema from chat completions.
It does not use a `choices` array. Provider-aware SSE text extraction MUST
handle it as a distinct format under the OpenAI provider.

| Event type (SSE `event:` field) | Content field | Requirement |
|---|---|---|
| `response.output_text.delta` | `delta` (top-level string) | MUST restore |
| `response.output_text.done` | `text` (top-level string) | MUST restore |
| `response.reasoning_summary_text.delta` | `delta` (top-level string) | MUST restore |

Responses API events MUST be distinguished from chat completion events by
their SSE `event:` line. Chat completions either omit the `event:` line or
use a non-`response.*` event type. A Responses API event is any event whose
`event:` value starts with `response.`.

When a Responses API stream is detected, the chat-completion field paths
(`choices[0].delta.*`) MUST NOT be applied — they would silently find nothing
and leave all fakes unrestored.

##### Gemini

Content appears in the `candidates[0].content.parts` array. Unlike other
providers, a single Gemini event MAY include multiple parts.

| Content field | Requirement |
|---|---|
| `candidates[0].content.parts[N].text` | MUST restore (all N) |
| `candidates[0].content.parts[N].functionCall.args.*` | MUST restore (string values only) |
| `candidates[0].content.parts[N].codeExecutionResult.output` | MUST restore |
| `candidates[0].content.parts[N].executableCode.code` | SHOULD restore |
| `thoughtSignature` (top-level) | MUST NOT modify |
| `groundingMetadata` (top-level) | MUST NOT modify |

When `includeThoughts: true` is set in the request, the `parts` array MAY contain
thought content in earlier indices and answer content in later indices. Restore MUST
apply to ALL parts with a `text` field regardless of array index — not only
`parts[0]`. E2E tests MUST set `includeThoughts: true` to exercise this path.

For `functionCall.args`, restore MUST apply to all string-typed values within
the `args` object. A secret appearing as a string value in a JSON object is a
contiguous byte sequence in the concatenated text buffer — no JSON-aware traversal
is required; the standard byte-level restore guarantee (buffering until no partial
match is possible) is sufficient. Non-string values MUST NOT be modified.

#### Multi-Field Accumulation

A single response stream MAY contain multiple independent content-bearing
fields (e.g., an Anthropic response with both `thinking_delta` and
`text_delta` blocks, or an OpenAI response with `content` and `tool_calls`).
Each distinct content field MUST maintain its own independent accumulation
buffer. Fakes MUST NOT be matched across the boundaries of different content
fields.

#### Terminal Event Ordering

Non-content SSE frames that signal the end of a content block or the entire
response MUST NOT be forwarded to the downstream client until all held content
for the scope they terminate has been completely flushed. Forwarding a terminal
event while content remains in the hold window inverts the protocol-mandated
ordering: the client finalizes a content block before receiving its complete
text, causing downstream parse errors on truncated content.

Two scopes apply:

- **Block-scope terminal**: terminates a specific content block. Before
  forwarding, all accumulation buffers whose `FieldKey` is scoped to that
  block MUST be completely flushed. Buffers for other active blocks are
  unaffected.

- **Stream-scope terminal**: terminates the entire response. Before
  forwarding, ALL active accumulation buffers MUST be completely flushed.

"Complete flush" in this context means a hold window of zero: all held bytes
are processed and emitted as synthetic frames regardless of `max_fake_len`.
This differs from the normal safe-prefix flush (hold window = `max_fake_len`),
which only emits the prefix that cannot contain a fake boundary.

##### Anthropic terminal events

| Event (`type` field) | Scope | Buffers flushed |
|---|---|---|
| `content_block_stop` (with `index` = N) | Block | All `AnthropicDelta` keys where `index == N` |
| `message_delta` | Stream | All |
| `message_stop` | Stream | All |

`ping`, `message_start`, and `content_block_start` are non-terminal events.
They carry no accumulated content at the time they arrive — `message_start`
precedes any delta, and `content_block_start` precedes the first delta for its
block — and MUST NOT trigger a flush.

##### OpenAI (Chat Completions) terminal events

| Signal | Scope | Buffers flushed |
|---|---|---|
| Data frame where `choices[0].finish_reason` is non-null | Stream | All |
| `[DONE]` (non-JSON `data:` value) | Stream | All |

The finish-reason frame carries an empty `delta` object and no extractable
content fields. It MUST still trigger a complete flush of all accumulators
before being forwarded.

##### OpenAI (Responses API) terminal events

Live observation of the real Responses API event sequence:
`response.output_text.delta` ×N → `response.output_text.done` →
`response.content_part.done` → `response.output_item.done` → `response.completed`.

| Event type (`event:` line) | Scope | Buffers flushed |
|---|---|---|
| `response.content_part.done` | Stream | All |
| `response.output_item.done` | Stream | All |
| `response.completed` | Stream | All |
| `response.failed` | Stream | All |
| `response.cancelled` | Stream | All |
| `response.incomplete` | Stream | All |

`response.created`, `response.in_progress`, `response.output_item.added`,
and `response.content_part.added` are non-terminal metadata events; they
arrive before content accumulation begins for their associated item and
MUST NOT trigger a flush.

##### Gemini terminal events

Live observation: Gemini sends `finishReason` **co-located with content**
in the same `data:` frame. That frame has extractable content fields, so it
goes through the content-accumulation path (path B) — not the passthrough
path (path A) — and no ordering inversion occurs in practice.

A frame where `candidates[0].finishReason` is non-null AND
`extract_fields` returns empty (no content parts) constitutes a stream-scope
terminal and MUST trigger a complete flush before forwarding. This covers
error or edge-case frames that carry only a finish signal.

| Signal | Scope | Buffers flushed |
|---|---|---|
| Frame with non-null `finishReason` and no extractable content | Stream | All |

#### Multi-Field Accumulation

A single response stream MAY contain multiple independent content-bearing
fields (e.g., an Anthropic response with both `thinking_delta` and
`text_delta` blocks, or an OpenAI response with `content` and `tool_calls`).
Each distinct content field MUST maintain its own independent accumulation
buffer. Fakes MUST NOT be matched across the boundaries of different content
fields.

#### Verifiable Conditions

**VC-SSE-1 (Anthropic thinking).** An Anthropic response containing a swapped
fake split across `thinking_delta` events (`delta.thinking` field) MUST have
the fake fully restored in the output.

**VC-SSE-2 (Anthropic tool use).** An Anthropic response containing a swapped
fake split across `input_json_delta` events (`delta.partial_json` field) MUST
have the fake fully restored in the output.

**VC-SSE-3 (Anthropic signature passthrough).** An Anthropic `signature_delta`
event MUST pass through with its `delta.signature` field byte-for-byte
unmodified, even if the signature bytes coincidentally match a fake pattern.

**VC-SSE-4 (OpenAI tool calls).** An OpenAI chat completion response
containing a swapped fake split across `tool_calls[0].function.arguments`
delta events MUST have the fake fully restored.

**VC-SSE-5 (OpenAI/OpenRouter reasoning).** An OpenAI or OpenRouter response
containing a swapped fake split across `reasoning_content` delta events MUST
have the fake fully restored.

**VC-SSE-6 (OpenAI deprecated function_call).** An OpenAI response containing
a swapped fake split across `function_call.arguments` delta events MUST have
the fake fully restored.

**VC-SSE-6b (OpenAI refusal).** An OpenAI response containing a swapped fake
in the `choices[0].delta.refusal` field MUST have the fake fully restored.

**VC-SSE-7 (Responses API text).** An OpenAI Responses API stream
(`v1/responses`) containing a swapped fake split across
`response.output_text.delta` events MUST have the fake fully restored. The
corresponding `response.output_text.done` event MUST also contain restored
text.

**VC-SSE-8 (Responses API reasoning).** An OpenAI Responses API stream
containing a swapped fake split across
`response.reasoning_summary_text.delta` events MUST have the fake fully
restored.

**VC-SSE-9 (Gemini multi-part text).** A Gemini response with thought content
in `parts[0].text` and answer content in `parts[1].text` MUST restore fakes
in both parts, not only `parts[0]`.

**VC-SSE-10 (Gemini code execution output).** A Gemini response containing a
swapped fake in `codeExecutionResult.output` MUST have the fake fully
restored.

**VC-SSE-11 (Gemini tool call args).** A Gemini response containing a swapped
fake in string values of `functionCall.args` MUST have the fake fully
restored. Non-string arg values MUST be byte-for-byte unmodified.

**VC-SSE-12 (Gemini metadata passthrough).** Gemini `thoughtSignature` and
`groundingMetadata` fields MUST pass through byte-for-byte unmodified.

**VC-SSE-13 (cross-field isolation).** When a response contains multiple
independent content fields (e.g., both `delta.content` and
`tool_calls[0].function.arguments`), fakes in each field MUST be restored
independently. A partial fake at the end of one field's accumulation MUST NOT
match against text from a different field.

**VC-SSE-14 (Anthropic block-stop ordering).** Given a fake whose bytes span
multiple `input_json_delta` events for content block index N such that some
bytes remain in the hold window when `content_block_stop` for index N arrives:
all synthetic frames carrying the remaining restored content for block N MUST
appear in the output before the `content_block_stop` frame.

**VC-SSE-15 (Anthropic message-stop ordering).** Given any accumulated content
remaining in the hold window when `message_stop` arrives: all synthetic frames
carrying the restored content MUST appear in the output before the
`message_stop` frame.

**VC-SSE-16 (Anthropic block-stop isolation).** Given two simultaneous active
accumulation buffers for block index 0 and block index 1: when
`content_block_stop` for index 0 arrives, the buffer for index 1 MUST NOT be
flushed. Only block 0's buffer is flushed before the stop is forwarded.

**VC-SSE-17 (OpenAI chat finish-reason ordering).** Given accumulated tool
arguments or reasoning content remaining in the hold window when the frame
with non-null `choices[0].finish_reason` arrives: all synthetic frames
carrying the restored content MUST appear before the finish-reason frame.
The `[DONE]` signal MUST appear after all restored content frames.

**VC-SSE-18 (Responses API content_part.done ordering).** Given accumulated
content remaining in the hold window when `response.content_part.done`
arrives: all synthetic frames carrying the restored content MUST appear
before the `response.content_part.done` frame. The same ordering constraint
applies to `response.output_item.done` and `response.completed`: each MUST
appear after all preceding restored content frames.

**VC-SSE-19 (Gemini finishReason co-location).** When a Gemini frame carries
both content parts and a non-null `finishReason`, it MUST be processed as a
content-accumulation frame (path B). No ordering inversion occurs because
the terminal signal is in the same frame as the content it terminates.

**VC-SSE-20 (Gemini empty-terminal ordering).** If a Gemini frame carries a
non-null `finishReason` and no extractable content (extract_fields returns
empty), any accumulated content from prior frames MUST be emitted before
that frame is forwarded.

## Known Limitations

### Gemini terminal metadata loss in synthetic frames

When the SSE restore stream is active (at least one secret was detected in the
request body), Gemini's `finishReason`, `usageMetadata`, and `modelVersion`
fields are not preserved in the synthetic frames emitted by the restore
stream. In practice, Gemini sends these fields co-located with the final
content in a single `data:` frame. The restore stream accumulates the content,
emits restored synthetic frames (which do not include `finishReason` or
`usageMetadata`), and the original frame data carrying those fields is
discarded. Downstream clients that depend on `finishReason` in the SSE stream
will not receive it when the restore stream is active.

This is an accepted limitation of the current synthetic-frame architecture.
It does not affect cache correctness — the cached content contains the
restored text — but it does affect streaming clients that inspect
`finishReason` mid-stream. The limitation does not apply on cache hits
(the stored exchange does not include SSE framing) or when no secret is
detected (the restore stream is bypassed entirely).
