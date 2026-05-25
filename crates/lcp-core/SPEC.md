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

The provider registry recognizes four providers, each identified by a URL
prefix and paired with a default upstream base URL:

| Provider | URL prefix | Default upstream |
|---|---|---|
| Anthropic | `anthropic` | `https://api.anthropic.com` |
| OpenAI | `openai` | `https://api.openai.com` |
| OpenRouter | `openrouter` | `https://openrouter.ai/api` |
| Gemini | `gemini` | `https://generativelanguage.googleapis.com` |

The provider registry MUST support:
- Resolving a URL prefix string to a provider identity, returning an absent
  result if the prefix is unrecognized.
- Returning the canonical URL prefix for a given provider.
- Returning the default upstream base URL for a given provider.
- Providing the provider-specific set of request body fields to strip during
  normalization, beyond the fields stripped for all providers.

## Cache Key

The key derivation function MUST accept a provider identity, HTTP method, URL
path, and request body as inputs, and MUST produce a deterministic string
output — a BLAKE3 hex digest of `method + "|" + path + "|" + normalized_body`.
Normalization is provider-aware: the set of fields stripped from the body
depends on the provider.

> **Implementation note:** Provider-aware normalization is a planned extension.
> The current implementation strips only `stream` and is provider-unaware.
> The rules below describe the target behavior.

Normalization rules (applied to the request body):
1. Parse as JSON. On failure, use the raw byte sequence verbatim.
2. Strip transport-only fields common to all providers:
   - `stream` — affects transport only, not model output.
3. Strip the provider-specific attribution and routing fields listed below.
4. Sort all JSON object keys recursively (depth-first).
5. Re-serialize to compact JSON.

Per-provider attribution and routing fields that MUST be stripped during
normalization:

| Provider | Additional fields stripped | Reason |
|---|---|---|
| Anthropic | `metadata` (entire object, any depth) | User attribution; no effect on model output |
| OpenAI | `user` | User attribution; no effect on model output |
| OpenRouter | `user`, `provider`, `route` | User attribution and routing preference; see note below |
| Gemini | _(none)_ | No known attribution or routing fields in body |

> **Note on OpenRouter `provider` and `route`:** These fields SHOULD be stripped
> as an accepted approximation. When OpenRouter fallback routing selects a
> different model variant, the cached response may not be byte-identical but
> remains semantically equivalent for caching purposes.

Fields that MUST NOT be stripped:
- `model` — primary cache discriminator.
- `temperature`, `top_p`, and all other sampling parameters — affect output distribution.
- `seed` — controls deterministic output.
- `transforms` (OpenRouter) — rewrites prompt content before forwarding; semantic field.
- `thinking` (Anthropic) — controls extended chain-of-thought reasoning; affects response structure and content.
- `reasoning` (OpenRouter) — controls extended reasoning; affects response structure and content.

HTTP headers MUST NOT contribute to the key.

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
- **`FullEntry`** (complete row including exchange data): all `CacheEntry` fields plus
  `content_type: String`, `request: RequestRecord`, `chunks: Vec<ResponseChunk>`.
- **`CacheStats`**: `hits`, `misses`, `bytes_served_from_cache` (from the
  stats counters), `entries` (live count), `by_model` (entry count keyed by the
  model identifier extracted by the proxy layer; extraction may come from the
  request body's `model` field or from the URL path depending on the provider.
  Provider-qualified names such as `"anthropic/claude-sonnet-4"` appear as-is
  when stored that way).

### Behavioral contract

**Read (`get`)**: returns the stored `Exchange` on a non-expired hit, `None`
on a miss or expired entry. On a hit: increments `hit_count` and the `hits`
and `bytes_served_from_cache` stats counters. On a miss: increments the
`misses` counter. Expiry is lazy — expired entries are not deleted on read.
TTL of 0 means entries never expire.

**Write (`put`)**: stores the exchange, overwriting any existing entry for
the same key. `resp_bytes` MUST be computed as the sum of all chunk data
lengths.

**Tracing (`record_trace`, `get_trace`, `inspect_trace`)**: `record_trace(trace_id, cache_key)`
persists the association; `get_trace(trace_id)` returns the associated `CacheEntry` rows
ordered by `created_at`; `inspect_trace(trace_id)` returns the associated `FullEntry` rows
ordered by `created_at` with no side effects.

**Inspect (`inspect`)**: `inspect(key)` returns the `FullEntry` for a given key, or `None`
if absent. MUST NOT increment any counter or modify any row (read-only, no side effects).

**Admin (`clear_entries`, `clear_stats`, `stats`, `list_entries`)**: these
expose aggregate counters and support the admin endpoints in `lcp-server`.
`clear_entries` removes all cache and trace rows; it MUST NOT touch stats
counters. `clear_stats` resets counters to zero; it MUST NOT affect entries
or per-entry `hit_count` values.

## Known Limitations

- No cache eviction policy; storage grows until `clear_entries` is called.
- Stats counters are best-effort: a crash between an exchange operation and
  its counter increment may leave counters slightly off.
