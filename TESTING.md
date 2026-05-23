# Testing Strategy

## Principles

1. **Specs before tests.** Each component gets a `SPEC.md` that documents its
   observable contract. Tests assert that spec. If spec and behavior disagree,
   fix one or the other — never silently accept drift.

2. **No real upstream calls in unit/integration tests.** A `MockUpstream`
   server (axum or simple TcpListener) scripts exact response sequences. Tests
   are fully deterministic and network-free.

3. **E2E tests are opt-in.** Real upstream calls are gated behind
   `--features test-e2e` and require API keys in the environment.

---

## Test Tiers

**Inline unit tests** — `#[cfg(test)]` modules inside `src/`
- Test implementation internals: private functions, algorithms, data structures.
- Coupled to the implementation — change freely during refactoring.
- Not a behavioral contract.

**External tests** — Rust integration test binaries in `tests/`
- Test observable behavior at public API boundaries.
- Coupled to the spec, not the implementation.
- A failing external test is a bug or a deliberate spec change, never a
  refactor casualty.

| Tier | Entry point | Subdirectory | What it covers |
|---|---|---|---|
| **Spec invariants** | `tests/spec.rs` | `tests/spec/` | Direct MUST/SHOULD assertions from SPEC.md; must pass on every commit |
| **Integration** | `tests/integration.rs` | `tests/integration/` | Proxy handler, cache hit/miss flow, stats endpoints, provider routing |
| **E2E** | `tests/e2e.rs` | `tests/e2e/` | Full real upstream calls; gated by `--features test-e2e` |

### Run commands

```sh
cargo nextest run --lib                                  # unit only (all crates)
cargo nextest run --test spec                            # spec invariants
cargo nextest run --test integration                     # integration
cargo nextest run --features test-e2e --test e2e         # e2e
cargo nextest run --tests                                # all tiers (no e2e without feature)
cargo nextest run                                        # everything
```

---

## What to Test

### lcp-core

**hash.rs** (inline unit tests — already present):
- Identical bodies → same key
- `stream` field stripped before hashing
- Key-order independence (sorted keys produce same hash)
- Different content → different key
- Different paths → different key
- Non-JSON body → raw bytes hashed stably

**cache.rs** (inline unit tests — already present):
- Miss on empty cache
- Put → get roundtrip: status, content-type, chunks preserved
- Hit count increments on each get
- Stats track hits and misses correctly
- `clear_entries` removes all rows, returns count
- TTL: entry beyond TTL returns None (miss)

### lcp-server — spec invariants

These tests use a `MockUpstream` (a real HTTP server started per test) and
call the proxy handler directly via `axum::test_helpers` or via a bound port.

**Provider routing:**
- Request to `/anthropic/...` forwarded to anthropic upstream
- Request to `/openai/...` forwarded to openai upstream
- Request to `/openrouter/...` forwarded to openrouter upstream
- Request to `/unknown/...` returns 404

**Cache miss flow:**
- First request to `/anthropic/v1/messages` → upstream receives request,
  response forwarded, `x-lcp-cache: MISS` header present
- Response stored in cache after miss

**Cache hit flow:**
- Second identical request → upstream NOT called, response served from cache,
  `x-lcp-cache: HIT` header present
- Content matches original response

**Cache key semantics:**
- Two requests differing only in `stream` field → same cache key, second is a hit
- Two requests with different JSON key ordering but same content → same key, hit
- Two requests with different `messages` content → different keys, both miss

**Bypass header:**
- Request with `x-lcp-bypass: 1` → upstream called, response NOT stored

**Stats endpoints:**
- `GET /stats` returns JSON with `hits`, `misses`, `entries` fields
- `DELETE /cache` empties entries, returns `cleared_entries` count
- `DELETE /stats` resets counters

**Header forwarding:**
- `host`, `connection`, `transfer-encoding`, `accept-encoding`, `content-length`
  are stripped before forwarding; all others pass through

**Non-2xx responses:**
- Upstream 429 → forwarded to client, NOT stored in cache

### lcp-server — integration

- Start a full server (bound to random port), fire HTTP requests via reqwest
- Hit/miss cycle across multiple providers
- Stats accumulate correctly across concurrent requests

### E2E (opt-in, `--features test-e2e`)

- Real Anthropic call: `POST /anthropic/v1/messages` → 200, response cached
- Second identical call: `x-lcp-cache: HIT`, body matches
- Real OpenAI call (if `OPENAI_API_KEY` set)
- Real OpenRouter call (if `OPENROUTER_API_KEY` set)

---

## Test Infrastructure

### MockUpstream

A simple axum server that serves scripted responses:

```rust
MockUpstream::new()
    .respond(200, "text/event-stream", "data: hello\n\n")
    .start()
    .await
```

Lives in `tests/common/mock_upstream.rs`.

### Test layout

```
crates/lcp-core/
  src/hash.rs          # inline unit tests
  src/cache.rs         # inline unit tests

crates/lcp-server/
  tests/
    common/
      mod.rs
      mock_upstream.rs
    spec.rs
    spec/
      routing.rs
      cache_flow.rs
      headers.rs
      stats.rs
    integration.rs
    integration/
      proxy.rs

crates/lcp/
  tests/
    e2e.rs             # gated by #[cfg(feature = "test-e2e")]
```

### Key dev-dependencies

```toml
[dev-dependencies]
tokio = { workspace = true }
reqwest = { workspace = true }
axum = { workspace = true }
tempfile = "3"
serde_json = { workspace = true }
```
