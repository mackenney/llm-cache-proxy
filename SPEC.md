# lcp Specification

> The key words MUST, SHOULD, SHOULD NOT, MAY are used per RFC 2119.

## Purpose

`lcp` is a localhost HTTP proxy that intercepts LLM API calls, caches
successful responses on disk, and replays them on subsequent identical
requests. The primary goals are eliminating repeated API spend during
iterative development and providing a queryable log of exchanges for
offline replay and inspection.

## Non-Goals (v1)

- **System-wide transparent interception** (TLS MITM, `/etc/hosts` redirect,
  iptables redirect) — clients must point at lcp explicitly via an env var.
- **Faithful chunk-timing replay** — cache hits return the full response body
  in one shot; per-token streaming cadence is not preserved.
- **Request/response interceptor plugins** (secret redaction, body mutation) —
  deferred to a future extension point.
- **Authentication / multi-user isolation** — lcp is a single-user local tool;
  it has no auth and MUST NOT be exposed on a public interface.
- **TLS termination** — lcp speaks plain HTTP; clients set
  `ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic` (http, not https).

## Providers

lcp routes requests to three upstream providers identified by URL prefix:

| Prefix | Default upstream | Env-var override |
|---|---|---|
| `anthropic` | `https://api.anthropic.com` | `LCP_ANTHROPIC_UPSTREAM` |
| `openai` | `https://api.openai.com` | `LCP_OPENAI_UPSTREAM` |
| `openrouter` | `https://openrouter.ai/api` | `LCP_OPENROUTER_UPSTREAM` |

A client sets:
```
ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic
OPENAI_BASE_URL=http://127.0.0.1:9001/openai
OPENROUTER_BASE_URL=http://127.0.0.1:9001/openrouter  # also covers Gemini via OpenRouter
```

An unknown prefix MUST return HTTP 404.

## Cache Key

The cache key is a BLAKE3 hex digest of the normalized request body,
combined with the HTTP method and full path.

### Normalization rules

1. Parse the request body as JSON. If parsing fails, use the raw byte
   sequence as-is.
2. Strip semantic-free fields: `stream`. These fields alter transport
   behavior but not the logical response.
3. Sort all JSON object keys recursively (depth-first).
4. Serialize back to compact JSON.

The method and path are concatenated with `|` separators before hashing:
```
blake3(method + "|" + path + "|" + normalized_body)
```

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

## Proxy Behavior

### Cache miss

1. Forward the request to the upstream, preserving method, path, query
   string, and all headers except `host`, `connection`, `transfer-encoding`,
   `accept-encoding`, and `content-length` (reqwest manages these).
2. Stream the response body back to the client as it arrives.
3. If the response status is `2xx`, store the exchange in the cache keyed by
   the cache key computed above. The stored entry includes: status, content-type,
   ordered response chunks with arrival timestamps (ms since first chunk), and
   request metadata.
4. The response MUST include `x-lcp-cache: MISS` and `x-lcp-key: <first-12-chars>`.

### Cache hit

1. Serve the stored response body to the client.
2. The response MUST include `x-lcp-cache: HIT` and `x-lcp-key: <first-12-chars>`.
3. The `hit_count` for the entry MUST be incremented by 1.

### Bypass

A request with header `x-lcp-bypass: 1` MUST skip both the cache read and
the cache write. The request is forwarded as-is, and the response is returned
without storing.

## SSE / Streaming

lcp buffers the full upstream response before forwarding it to the client.
On cache hit, the stored body is returned in one shot.

> **Consequence**: clients that rely on streaming token-by-token see the
> full response arrive instantly on cache hits (and with a slight delay on
> misses due to buffering). This is acceptable for development use cases.
> Faithful timing replay is a non-goal for v1.

`accept-encoding` MUST be stripped from forwarded requests so that upstream
providers do not apply compression to SSE streams.

## Stats and Admin Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/` | Health check. Returns `{"status":"ok"}`. |
| `GET` | `/stats` | Hit/miss counts, bytes served, entries, by-model breakdown. |
| `DELETE` | `/stats` | Reset stat counters. Entries are unaffected. |
| `DELETE` | `/cache` | Delete all cache entries. Stats are unaffected. |

## Configuration

All options are available as CLI flags and matching env vars. CLI flags take
priority over env vars.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--port` | `LCP_PORT` | `9001` | Listen port |
| `--host` | `LCP_HOST` | `127.0.0.1` | Bind host |
| `--db` | `LCP_DB` | `~/.cache/lcp/cache.db` | SQLite path |
| `--ttl` | `LCP_TTL` | `0` | Entry TTL in seconds (0 = forever) |
| `--anthropic-upstream` | `LCP_ANTHROPIC_UPSTREAM` | see table above | Anthropic upstream override |
| `--openai-upstream` | `LCP_OPENAI_UPSTREAM` | see table above | OpenAI upstream override |
| `--openrouter-upstream` | `LCP_OPENROUTER_UPSTREAM` | see table above | OpenRouter upstream override |

## Future: Interceptor Plugin API

A future version will expose a plugin extension point that runs before the
cache key is computed and before the response is stored. Intended use cases:

- Secret redaction: strip or hash API keys, user IDs, and other PII from
  stored request/response bodies before writing to disk.
- Request mutation: normalize or enrich requests before forwarding.
- Response filtering: drop or redact fields from stored responses.

The plugin API is intentionally out of scope for v1.

## Per-Component Specs

Each crate MUST have a `SPEC.md` before implementation begins:

```
crates/lcp-core/SPEC.md    types, hashing, cache contract
crates/lcp-server/SPEC.md  proxy behavior, routing, SSE contract
```
