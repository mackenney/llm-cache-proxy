# Step 03: Refactor Cache Miss Path with Spawn+Channel

## Objective
Replace the buffered response pattern for cache misses with true streaming using a spawn+channel architecture:
- A spawned task reads upstream chunks and forwards them through a channel
- The channel receiver becomes the response body via `Body::from_stream()`
- Cache write happens in the spawned task after stream completes successfully

## Why
- Eliminates full-body buffering before response (spec violation)
- Client receives first byte as soon as upstream sends it (TTFB improvement)
- Natural backpressure via bounded channel
- Cache write happens atomically after stream completes

## Depends On
- step-01 (ReceiverStream adapter and imports must be present)

## File to Modify
`crates/lcp-server/src/proxy.rs`

## Current Code to Replace (lines 107-177)

The current code from `let model = extract_model(&body);` through the response builder needs replacement. This includes:
- Lines 108-129: chunk buffering loop
- Lines 131-156: cache write
- Lines 158-177: response building

## New Implementation

Replace lines 107-177 with the following:

```rust
    let model = extract_model(&body);
    let do_cache = !bypass && status.is_success();

    // Create channel for streaming response to client
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    // Clone values needed by the spawned task
    let cache = state.config.cache.clone();
    let key_clone = key.clone();
    let full_path_clone = full_path.clone();
    let body_clone = body.clone();
    let trace_id_clone = trace_id.clone();
    let provider_prefix = provider.path_prefix().to_owned();
    let model_clone = model.clone();
    let content_type_clone = content_type.clone();
    let status_code = status.as_u16();

    // Spawn task to read upstream, forward to client channel, and cache on completion
    tokio::spawn(async move {
        let mut chunks: Vec<ResponseChunk> = Vec::new();
        let mut stream = upstream_resp.bytes_stream();
        let start = Instant::now();

        let stream_complete = loop {
            match stream.next().await {
                Some(Ok(bytes)) => {
                    let offset_ms = start.elapsed().as_millis() as u64;
                    chunks.push(ResponseChunk {
                        offset_ms,
                        data: String::from_utf8_lossy(&bytes).into_owned(),
                    });
                    // Forward to client; break if client disconnected
                    if tx.send(Ok(bytes)).await.is_err() {
                        tracing::debug!("client disconnected mid-stream");
                        break false; // Don't cache partial responses
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!(err = %e, chunks = chunks.len(), "upstream stream error");
                    break false; // Don't cache on error
                }
                None => break true, // Stream completed successfully
            }
        };
        drop(tx); // Signal end-of-stream to receiver

        // Cache write only if stream completed successfully and caching is enabled
        if stream_complete && do_cache {
            let exchange = Exchange {
                request: RequestRecord {
                    method: "POST".into(),
                    path: full_path_clone,
                    body: String::from_utf8_lossy(&body_clone).into_owned(),
                },
                status: status_code,
                content_type: content_type_clone,
                chunks,
            };
            match cache.put(&key_clone, &provider_prefix, model_clone.as_deref(), &exchange) {
                Ok(()) => {
                    if let Some(ref tid) = trace_id_clone {
                        if let Err(e) = cache.record_trace(tid, &key_clone) {
                            tracing::warn!(err = %e, "failed to record trace on miss");
                        }
                    }
                }
                Err(e) => tracing::warn!(err = %e, "failed to cache exchange"),
            }
        }
    });

    // Build streaming response
    let body_stream = ReceiverStream::new(rx);
    let body = Body::from_stream(body_stream);

    let lcp_status = if bypass { "BYPASS" } else { "MISS" };
    let mut response = Response::builder()
        .status(status)
        .header("content-type", &content_type)
        .header("x-lcp-cache", lcp_status);

    if !bypass {
        response = response.header("x-lcp-key", &key[..12]);
    }

    if is_sse {
        response = response
            .header("cache-control", "no-cache")
            .header("transfer-encoding", "chunked");
    }

    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
```

## Key Design Decisions

### Spawn + Channel Pattern
- Decouples upstream reading from client write pace
- Spawned task owns cache write responsibility
- Backpressure via bounded channel (32 items)

### Caching Invariants
- `stream_complete` tracks whether the full stream was received
- Cache write only happens when `stream_complete && do_cache`
- Client disconnect → `tx.send().is_err()` → `stream_complete = false` → no cache
- Upstream error → `stream_complete = false` → no cache
- Partial responses are never cached

### Response Timing
- Response headers sent immediately (status, content-type, x-lcp-*)
- Body streams asynchronously as chunks arrive
- Client sees first byte as soon as upstream sends it

## Acceptance Criteria

1. **Compilation succeeds:**
   ```bash
   cargo build -p lcp-server 2>&1 | head -20
   ```
   Expected: no errors

2. **Existing tests pass:**
   ```bash
   cargo nextest run 2>&1 | tail -10
   ```
   Expected: all tests pass

3. **Clippy clean:**
   ```bash
   cargo clippy -p lcp-server 2>&1 | grep -E "^error" || echo "No errors"
   ```
   Expected: "No errors"

4. **Manual verification (optional but recommended):**
   ```bash
   # Start server (if test infrastructure supports it)
   # Make a request to a streaming endpoint
   # Verify response streams incrementally, not all-at-once
   ```

## Commit Message
```
step-03: stream cache misses with spawn+channel
```
