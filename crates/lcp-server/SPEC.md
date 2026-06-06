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
