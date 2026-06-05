# lcp Specification

> The key words MUST, SHOULD, SHOULD NOT, MAY are used per RFC 2119.

## Purpose

`lcp` is a localhost HTTP proxy that intercepts LLM API calls, caches
successful responses on disk, and replays **reasonably equivalent** responses
on subsequent matching requests. The primary goals are eliminating repeated
API spend during iterative development and providing a queryable log of
exchanges for offline replay and inspection.

**Equivalence** is model- and transport-relative: a cached response for a
given model and request is considered a valid replay even if minor formatting
or metadata details differ across API versions or streaming transport
variants. Per-token streaming cadence is intentionally not preserved on
replay.

## Non-Goals (v1)

- **System-wide transparent interception** (TLS MITM, `/etc/hosts` redirect,
  iptables redirect) — clients must point at lcp explicitly via an env var.
- **Arbitrary plugin execution** — no third-party interceptor plugins are loaded
  at runtime. The built-in doppel extension is the only extension that runs;
  the architecture exposes insertion points for future additions without
  structural changes.
- **Authentication / multi-user isolation** — lcp is a single-user local
  tool; it has no auth and MUST NOT be exposed on a public interface.
- **TLS termination** — lcp speaks plain HTTP; clients set
  `ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic` (http, not https).

## Providers

lcp routes requests to four upstream providers identified by URL prefix. All
proxied LLM API calls use HTTP POST.

| Prefix | Default upstream | Env-var override |
|---|---|---|
| `anthropic` | `https://api.anthropic.com` | `LCP_ANTHROPIC_UPSTREAM` |
| `openai` | `https://api.openai.com` | `LCP_OPENAI_UPSTREAM` |
| `openrouter` | `https://openrouter.ai/api/v1` | `LCP_OPENROUTER_UPSTREAM` |
| `gemini` | `https://generativelanguage.googleapis.com` | `LCP_GEMINI_UPSTREAM` |

A client sets:
```
ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic
OPENAI_BASE_URL=http://127.0.0.1:9001/openai
OPENROUTER_BASE_URL=http://127.0.0.1:9001/openrouter
GEMINI_BASE_URL=http://127.0.0.1:9001/gemini
```

An unknown prefix MUST return HTTP 404.

## Cache Key

The cache key is a BLAKE3 hex digest derived from the provider, model, and
normalized request body. The provider is implicit in the path prefix
(e.g. `/anthropic/v1/messages`). The model is a top-level field in the
request body and MUST NOT be stripped during normalization — it is a
first-class cache discriminator.

Hash input:
```
blake3(method + "|" + path + "|" + normalized_body)
```

### Normalization rules

Normalization rules — including per-provider strip lists, MUST NOT strip
constraints, and ordering — are defined authoritatively in
`crates/lcp-core/SPEC.md`.

Headers are NOT included in the key. API key rotation MUST NOT bust the
cache.

## Cache Storage

- Storage: a single SQLite file.
- Default location: `$XDG_CACHE_HOME/lcp/cache.db` (falls back to
  `~/.cache/lcp/cache.db`).
- Overridden by `--db <path>` or `LCP_DB`.
- Only `2xx` responses are cached. Non-2xx responses are forwarded as-is
  and MUST NOT be stored.
- TTL: configurable via `--ttl <seconds>`. `0` means entries never expire.
  Expired entries MUST be treated as misses on read; they are not deleted
  proactively (lazy expiry on access).

### Schema (logical)

```
entries(
  key          TEXT PRIMARY KEY,
  status       INTEGER,
  content_type TEXT,
  provider     TEXT,        -- routing prefix (anthropic, openai, openrouter, gemini)
  model        TEXT,        -- top-level `model` field from request body
  exchange_json TEXT,       -- JSON array of {data: string, offset_ms: u64}
  req_bytes    INTEGER,     -- total bytes of the original request body
  resp_bytes   INTEGER,     -- total bytes across all chunks (computed at store time)
  created_at   TEXT,        -- ISO-8601 UTC timestamp
  hit_count    INTEGER      -- incremented on every cache hit
)

-- offset_ms is ms since first chunk arrived; recorded for observability,
-- NOT used to pace replay.

trace_entries(
  trace_id  TEXT,
  cache_key TEXT,
  PRIMARY KEY (trace_id, cache_key)
)
-- many-to-many: a trace may span many cache entries;
-- a cache entry may appear in many traces.

stats(
  k TEXT PRIMARY KEY,  -- e.g. 'hits', 'misses', 'bytes_served_from_cache'
  v INTEGER NOT NULL
)
-- Accumulates counters independently of entries.
-- Survives DELETE /cache; reset by DELETE /stats.
-- Counters: 'hits', 'misses', 'bytes_served_from_cache'.
```

## Proxy Behavior

### Cache miss

1. Forward the request to the upstream, preserving method, path, query
   string, and all headers except `host`, `connection`, `transfer-encoding`,
   `accept-encoding`, `content-encoding`, and `content-length` (reqwest manages these).
2. Stream each response chunk back to the client as it arrives from
   upstream; do not buffer the full response before forwarding.
3. If the response status is `2xx`, store the exchange keyed by the cache
   key computed above. The stored entry includes: status, content-type,
   ordered response chunks with arrival timestamps (`offset_ms` since first
   chunk), and request metadata.
4. The response MUST include `x-lcp-cache: MISS` and
   `x-lcp-key: <first-12-chars-of-key>`. These headers are added for
   **all** upstream responses, including non-`2xx`.
5. The `stats` table MUST be updated: increment `misses` by 1. This
   applies to all upstream responses, including non-`2xx`.

### Cache hit

1. Replay the stored response chunks to the client sequentially at full
   speed — no artificial inter-chunk delay. Chunk boundaries from the
   original response are preserved.
2. The response MUST include `x-lcp-cache: HIT` and
   `x-lcp-key: <first-12-chars-of-key>`.
3. The `hit_count` for the entry MUST be incremented by 1.
4. The `stats` table MUST be updated: increment `hits` by 1 and
   `bytes_served_from_cache` by the entry's `resp_bytes`.

### Bypass

A request with header `x-lcp-bypass: 1` MUST skip both the cache read and
the cache write. The request is forwarded as-is and the response is returned
without storing or tracing. The response MUST include `x-lcp-cache: BYPASS`;
the `x-lcp-key` header MUST NOT be present on bypass responses.

### Consumer compression

Downstream clients MAY send `Accept-Encoding` headers (gzip, br, zstd,
etc.). The proxy MUST:

1. Strip `Accept-Encoding` before forwarding to the upstream so that
   upstream responses arrive uncompressed.
2. Accept and decompress any compressed request body sent by the downstream
   client before hashing and forwarding.
3. Strip `Content-Encoding` from forwarded requests after decompression.
   The upstream MUST always receive a plaintext body with no
   `Content-Encoding` header, regardless of what the downstream sent.
   client before hashing and forwarding.
3. Return responses to the downstream client uncompressed; the proxy does
   not re-apply content encoding on the response path.

## SSE / Streaming

Streaming responses (SSE / `text/event-stream` and chunked JSON) are handled
transparently in both directions:

- **Cache miss**: chunks are forwarded to the client as they arrive; the
  proxy does not buffer the full response body.
- **Cache hit**: stored chunks are replayed sequentially at full speed
  without timing delay, preserving original chunk boundaries. Downstream
  consumers receive a properly chunked SSE stream.

`Accept-Encoding` MUST be stripped from all forwarded requests so that
upstream providers do not apply compression to SSE streams.

When the swap/restore extension is active, Phase 3 MUST apply restoring
at the semantic SSE text level for streaming responses — raw byte-level
matching is insufficient because fakes are split across `data:` events.
See `crates/lcp-server/SPEC.md §SSE-Aware Restore`.

## Tracing

A client MAY attach a trace identifier to any request:

```
x-lcp-trace: <trace-id>
```

`<trace-id>` is an arbitrary client-supplied string (UUID or freeform
label).

### Behavior

- The proxy MUST persist the `(trace_id, cache_key)` pair in `trace_entries`
  for every cached (non-bypass) request that carries the header.
- Multiple requests in a session share the same trace ID.
- A single cache entry MAY appear in multiple trace sessions (many-to-many).
- Bypass requests (`x-lcp-bypass: 1`) are NOT recorded in `trace_entries`.

### Query endpoint

| Method | Path | Description |
|---|---|---|
| `GET` | `/trace/<trace-id>` | Metadata for all cache entries in the trace, ordered by `created_at`. |
| `GET` | `/trace/<trace-id>?full=true` | Same but each entry includes the full request body and response chunks. |

Default response shape:
```json
{
  "trace_id": "<trace-id>",
  "entries": [
    { "key": "...", "created_at": "...", "status": 200, "hit_count": 3 }
  ]
}
```

With `?full=true`, each entry also includes `provider`, `model`, `content_type`,
`req_bytes`, `resp_bytes`, `request: {method, path, body}`, and `chunks: [{data, offset_ms}]`.

## Stats and Admin Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/` | Health check. Returns `{"status":"ok"}`. |
| `GET` | `/stats` | Hit/miss counts, bytes served, total entries, by-model entry counts. |
| `DELETE` | `/stats` | Reset the `stats` table counters to zero. Per-entry `hit_count` values and cache entries are unaffected. |
| `DELETE` | `/cache` | Delete all cache entries and trace entries. The `stats` table counters are unaffected. |
| `GET` | `/cache/<key>` | Fetch the full exchange (request + response chunks) for a given cache key. `<key>` MUST be the full BLAKE3 hex digest — the 12-char `x-lcp-key` response header is an observability prefix only and is not accepted here. Returns 404 if not found. |

### `/cache/<key>` response shape

```json
{
  "key": "abc123def456",
  "created_at": "2026-05-24T02:00:00Z",
  "provider": "anthropic",
  "model": "claude-opus-4",
  "status": 200,
  "content_type": "text/event-stream",
  "hit_count": 3,
  "req_bytes": 142,
  "resp_bytes": 1024,
  "request": { "method": "POST", "path": "/anthropic/v1/messages", "body": "..." },
  "chunks": [
    { "data": "data: ...\n\n", "offset_ms": 0 },
    { "data": "data: ...\n\n", "offset_ms": 120 }
  ]
}
```

### `/stats` response shape

```json
{
  "hits": 312,
  "misses": 87,
  "bytes_served_from_cache": 4182404,
  "entries": 87,
  "by_model": {"anthropic/claude-sonnet-4": 46, "gpt-4o": 41}
}
```

`hits`, `misses`, and `bytes_served_from_cache` come from the `stats` table.
`entries` is `COUNT(*)` from `entries`.
`by_model` is `COUNT(*) GROUP BY model` from `entries`, keyed by the model
identifier extracted from each request. For body-based providers (Anthropic,
OpenAI, OpenRouter), this is the raw `model` field value from the request body.
For Gemini, this is the model name extracted from the URL path (e.g.,
`gemini-2.5-flash` from `/models/gemini-2.5-flash:generateContent`). Model
names that include a provider prefix (e.g., OpenRouter's
`"anthropic/claude-sonnet-4"`) appear as-is; names without a prefix appear
without one.

## Configuration

### Precedence

Options are resolved in the following order (highest wins):

1. CLI flag
2. Environment variable
3. Config file
4. Built-in default

### Config file

lcp reads a TOML config file on startup:

- Default path: `$XDG_CONFIG_HOME/lcp/config.toml` (falls back to `~/.config/lcp/config.toml`)
- Override with `--config <path>` or the `LCP_CONFIG` env var
- Missing files are silently ignored
- Keys match long flag names with hyphens replaced by underscores (e.g. `anthropic_upstream`)
- A malformed config file MUST emit a warning to stderr and be ignored entirely

### Flags

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--config` | `LCP_CONFIG` | `$XDG_CONFIG_HOME/lcp/config.toml` | Config file path |
| `--port` | `LCP_PORT` | `9001` | Listen port |
| `--host` | `LCP_HOST` | `127.0.0.1` | Bind host |
| `--db` | `LCP_DB` | `~/.cache/lcp/cache.db` | SQLite path |
| `--ttl` | `LCP_TTL` | `0` | Entry TTL in seconds (0 = forever) |
| `--timeout` | `LCP_TIMEOUT` | `300` | Upstream request timeout in seconds |
| `--anthropic-upstream` | `LCP_ANTHROPIC_UPSTREAM` | see table above | Anthropic upstream |
| `--openai-upstream` | `LCP_OPENAI_UPSTREAM` | see table above | OpenAI upstream |
| `--openrouter-upstream` | `LCP_OPENROUTER_UPSTREAM` | see table above | OpenRouter upstream |
| `--gemini-upstream` | `LCP_GEMINI_UPSTREAM` | see table above | Gemini upstream |
| `--print-config` | _(none)_ | _(flag)_ | Print effective config as TOML and exit |

Path values (`--db`, `secrets_file`) support `~` expansion: a leading `~` is
replaced with the user's home directory.

### Extension configuration

Extension options are set in the config TOML under `[extensions]` — they have
no CLI flag or env var equivalent.

#### `[extensions.doppel]`

Enables the built-in swap/restore extension (backed by `doppel`).

| Key | Default | Description |
|---|---|---|
| `secrets_file` | _(unset)_ | Path to an `doppel` TOML patterns file. `~` is expanded. |

Behaviour when the section is present:
- `secrets_file` absent → warning at startup, swapping disabled.
- `secrets_file` set, file missing or invalid → warning at startup, swapping disabled.
- `secrets_file` set, file valid → doppel extension loaded; all patterns in the file are active.

Create a patterns file: `doppel init --patterns <path>`.
Register secrets: `doppel register --patterns <path> --label <label>`.

Example:
```toml
[extensions.doppel]
secrets_file = "~/.config/lcp/secrets.toml"
```
## Extension Architecture

lcp runs an ordered extension pipeline on every proxied request. Extensions
implement the `Extension` trait and are registered with `ExtensionPipeline`.
Three phases fire per request:

| Phase | Hook | Fires | Purpose |
|---|---|---|---|
| 1 | `on_request_body` | every request, before cache key | normalize or inspect the body |
| 2 | `on_upstream_body` | cache miss only, before forwarding | transform the body sent to the upstream |
| 3 | `on_response_stream` | cache miss only, after upstream responds | wrap the response stream |

Phase 1 fires on every request including bypasses and cache hits; Phases 2 and 3
fire only on cache misses. An empty pipeline is a no-op.

The swap/restore extension (`DoppelExt`) is the only built-in extension. It is
opt-in via `[extensions.doppel]` in the config file. The per-phase behavior is
specified in `crates/lcp-server/SPEC.md §Extension Pipeline`.

## Per-Component Specs

Each crate MUST have a `SPEC.md` before implementation begins:

```
crates/lcp-core/SPEC.md    types, hashing, cache storage, tracing schema
crates/lcp-server/SPEC.md  proxy behavior, routing, SSE, tracing endpoints
```
