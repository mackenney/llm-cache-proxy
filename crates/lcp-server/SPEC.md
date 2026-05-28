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
(cache miss) or as they are read from storage (cache hit).

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
receives and what is written to the cache. For a scrub/unscrub extension pair,
Phase 3 restores the original bytes, so the cache stores originals while the
wire carried only fakes.
