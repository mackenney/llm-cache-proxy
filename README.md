# lcp — LLM Cache Proxy

> **⚠️ Internal tool — use with caution.**
> This project is at version 0.0.1, pre-release, and under active development.
> APIs, CLI flags, config keys, and cache formats **may change without notice**.
> It is not yet published to crates.io and carries no stability guarantees.

A local HTTP proxy that caches LLM API responses on disk and replays them on
subsequent identical requests. Stop paying for the same completion twice during
iterative development.

## How it works

lcp sits between your code and the LLM provider. On a cache miss it forwards
the request, streams the response back to you, and stores it. On a cache hit it
replays the stored response from disk at full speed. The cache key is a BLAKE3
hash of the provider, model, and normalized request body — API key rotation
never busts the cache.

## Quick start

```sh
cargo install --git https://github.com/mackenney/llm-cache-proxy crates/lcp

# Start the proxy (default port 9001)
lcp

# Point your LLM client at it
export ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic
export OPENAI_BASE_URL=http://127.0.0.1:9001/openai
export OPENROUTER_BASE_URL=http://127.0.0.1:9001/openrouter
export GEMINI_BASE_URL=http://127.0.0.1:9001/gemini
```

That's it. Your existing code works unchanged; lcp is transparent to the client.

## Configuration

All options accept a CLI flag, an env var, or a TOML config file entry.
Precedence: CLI flag > env var > config file > built-in default.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--port` | `LCP_PORT` | `9001` | Listen port |
| `--host` | `LCP_HOST` | `127.0.0.1` | Bind host |
| `--db` | `LCP_DB` | `~/.cache/lcp/cache.db` | SQLite cache path |
| `--ttl` | `LCP_TTL` | `0` (never expire) | Entry TTL in seconds |
| `--timeout` | `LCP_TIMEOUT` | `300` | Upstream timeout in seconds |
| `--anthropic-upstream` | `LCP_ANTHROPIC_UPSTREAM` | `https://api.anthropic.com` | |
| `--openai-upstream` | `LCP_OPENAI_UPSTREAM` | `https://api.openai.com` | |
| `--openrouter-upstream` | `LCP_OPENROUTER_UPSTREAM` | `https://openrouter.ai/api` | |
| `--gemini-upstream` | `LCP_GEMINI_UPSTREAM` | `https://generativelanguage.googleapis.com` | |
| `--config` | `LCP_CONFIG` | `~/.config/lcp/config.toml` | Config file path |

Print the current effective config as TOML (useful as a starter config file):

```sh
lcp --print-config
```

## Supported providers

| Prefix | Provider |
|---|---|
| `/anthropic` | Anthropic (Claude) |
| `/openai` | OpenAI |
| `/openrouter` | OpenRouter |
| `/gemini` | Google Gemini |

## Per-request headers

| Header | Effect |
|---|---|
| `x-lcp-bypass: 1` | Skip cache read and write for this request |
| `x-lcp-trace: <id>` | Tag this request with a trace ID for later inspection |

Responses always include `x-lcp-cache: HIT | MISS | BYPASS` and (on hits/misses)
`x-lcp-key: <first-12-chars-of-cache-key>`.

## Admin endpoints

```
GET  /           Health check
GET  /stats      Hit/miss counts, bytes served, entry count by model
DELETE /stats    Reset stats counters
DELETE /cache    Purge all cached entries
GET  /cache/<key>        Fetch a stored exchange by cache key
GET  /trace/<id>         List entries recorded under a trace ID
GET  /trace/<id>?full=true  Same with full request/response bodies
```

## Streaming

SSE and chunked responses are handled transparently. On a miss, chunks are
forwarded as they arrive. On a hit, stored chunks are replayed at full speed
with original boundaries preserved.
