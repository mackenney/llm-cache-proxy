# lcp-core Specification

> The key words MUST, MUST NOT, SHOULD, SHOULD NOT, MAY are used per RFC 2119.

## Purpose

`lcp-core` owns the shared domain primitives consumed by `lcp-server` and
`lcp`: the provider taxonomy, cache-key computation, and the persistent
exchange cache (storage + counters + tracing). No other crate re-implements
these.

## Non-Goals

- HTTP handling, request forwarding, SSE streaming.
- CLI argument parsing or process lifecycle.
- Any network I/O.

## Boundary with lcp-server

`lcp-server` depends on `lcp-core` for three things only:

1. **`Provider`** — to resolve a URL prefix to an upstream base URL.
2. **`cache_key`** — to derive a stable key from a request before touching
   the cache.
3. **`Cache`** — to read, write, and query cached exchanges.

All type definitions shared across the crate boundary (`Exchange`,
`ResponseChunk`, `RequestRecord`, `CacheEntry`, `CacheStats`) live in
`lcp-core`.

## Provider

`Provider` is a Rust `enum` with exactly four variants:

| Variant | URL prefix | Default upstream |
|---|---|---|
| `Anthropic` | `anthropic` | `https://api.anthropic.com` |
| `OpenAi` | `openai` | `https://api.openai.com` |
| `OpenRouter` | `openrouter` | `https://openrouter.ai/api` |
| `Gemini` | `gemini` | `https://generativelanguage.googleapis.com` |

`Provider` MUST expose:
- `from_prefix(s: &str) -> Option<Provider>` — parses a URL prefix.
- `path_prefix(self) -> &'static str` — returns the canonical prefix.
- `default_upstream(self) -> &'static str` — returns the default base URL.

## Cache Key

`cache_key(method: &str, path: &str, body: &[u8]) -> String` MUST return a
BLAKE3 hex digest of `method + "|" + path + "|" + normalized_body`.

Normalization rules (applied to `body`):
1. Parse as JSON. On failure, use the raw bytes (UTF-8 lossy) verbatim.
2. Strip the `stream` field at any depth — it affects transport only.
3. Sort all JSON object keys recursively.
4. Re-serialize to compact JSON.

The `model` field MUST NOT be stripped. HTTP headers MUST NOT contribute to
the key.

## Cache

`Cache` is a cloneable, `Send + Sync` handle to a SQLite-backed store. It
MUST be safe to share across async tasks without external locking.

### Stored types

- **`ResponseChunk`**: `data: String` (raw chunk bytes) + `offset_ms: u64`
  (ms since first chunk; observability only, never used to pace replay).
- **`RequestRecord`**: `method`, `path`, `body` (all `String`).
- **`Exchange`**: `request: RequestRecord`, `status: u16`,
  `content_type: String`, `chunks: Vec<ResponseChunk>`.
- **`CacheEntry`** (metadata row, no chunks): `key`, `provider`,
  `model: Option<String>`, `status: u16`, `hit_count`, `req_bytes`, `resp_bytes`,
  `created_at` (ISO-8601 UTC string).
- **`CacheStats`**: `hits`, `misses`, `bytes_served_from_cache` (from the
  stats counters), `entries` (live count), `by_model` (entry count keyed by the
  raw `model` field value; provider-qualified names such as `"anthropic/claude-sonnet-4"`
  appear as-is when stored that way in the request body).

### Behavioral contract

**Read (`get`)**: returns the stored `Exchange` on a non-expired hit, `None`
on a miss or expired entry. On a hit: increments `hit_count` and the `hits`
and `bytes_served_from_cache` stats counters. On a miss: increments the
`misses` counter. Expiry is lazy — expired entries are not deleted on read.
TTL of 0 means entries never expire.

**Write (`put`)**: stores the exchange, overwriting any existing entry for
the same key. `resp_bytes` MUST be computed as the sum of all chunk data
lengths.

**Tracing (`record_trace`, `get_trace`)**: `record_trace(trace_id, cache_key)`
persists the association; `get_trace(trace_id)` returns the associated
`CacheEntry` rows ordered by `created_at`.

**Admin (`clear_entries`, `clear_stats`, `stats`, `list_entries`)**: these
expose aggregate counters and support the admin endpoints in `lcp-server`.
`clear_entries` removes all cache and trace rows; it MUST NOT touch stats
counters. `clear_stats` resets counters to zero; it MUST NOT affect entries
or per-entry `hit_count` values.

## Known Limitations

- No cache eviction policy; storage grows until `clear_entries` is called.
- Stats counters are best-effort: a crash between an exchange operation and
  its counter increment may leave counters slightly off.
