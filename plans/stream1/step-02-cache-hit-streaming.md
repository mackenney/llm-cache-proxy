# Step 02: Refactor Cache Hit Path to Stream

## Objective
Convert `serve_cached()` from buffering all chunks into a single `Vec<u8>` to streaming chunks directly using `futures_util::stream::iter()` and `Body::from_stream()`.

## Why
- Preserves chunk boundaries from the original response (spec requirement)
- Zero-copy streaming: iterator adapters are lazy
- Enables TTFB for cache hits (first chunk delivered immediately)
- Independent of step-01 — uses `stream::iter()` which doesn't need `ReceiverStream`

## File to Modify
`crates/lcp-server/src/proxy.rs`

## Current Code (lines 179-193)
```rust
fn serve_cached(exchange: Exchange, key: &str) -> Response {
    let body: Vec<u8> = exchange
        .chunks
        .into_iter()
        .flat_map(|c| c.data.into_bytes())
        .collect();

    Response::builder()
        .status(exchange.status)
        .header("content-type", &exchange.content_type)
        .header("x-lcp-cache", "HIT")
        .header("x-lcp-key", &key[..12])
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
```

## New Code
Replace the entire `serve_cached` function with:

```rust
fn serve_cached(exchange: Exchange, key: &str) -> Response {
    // Stream chunks directly — preserves original chunk boundaries
    let chunk_stream = futures_util::stream::iter(
        exchange.chunks.into_iter().map(|c| {
            Ok::<_, std::io::Error>(Bytes::from(c.data))
        })
    );
    let body = Body::from_stream(chunk_stream);

    Response::builder()
        .status(exchange.status)
        .header("content-type", &exchange.content_type)
        .header("x-lcp-cache", "HIT")
        .header("x-lcp-key", &key[..12])
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
```

## Key Changes
1. **Remove `flat_map(...).collect()`** — no more concatenation into single buffer
2. **Use `stream::iter()`** — converts iterator into async stream
3. **Each chunk becomes `Bytes::from(c.data)`** — preserves chunk boundaries
4. **Use `Body::from_stream()`** — Axum streams the body lazily

## Invariant Preserved
"Chunk boundaries from original response are preserved" — each `ResponseChunk` in storage maps to one yield in the response stream.

## Acceptance Criteria

1. **Compilation succeeds:**
   ```bash
   cargo build -p lcp-server 2>&1 | head -20
   ```
   Expected: no errors

2. **Existing tests pass:**
   ```bash
   cargo nextest run -p lcp-server 2>&1 | tail -10
   ```
   Expected: all tests pass

3. **Clippy clean:**
   ```bash
   cargo clippy -p lcp-server 2>&1 | grep -E "^error" || echo "No errors"
   ```
   Expected: "No errors"

## Commit Message
```
step-02: stream cache hits with stream::iter
```
