# Step 03 — proxy.rs UTF-8 chunk corruption fix (B2)

**File:** `crates/lcp-server/src/proxy.rs`
**Wave:** 0 (no deps)

---

## B2 — from_utf8_lossy per-chunk corrupts multibyte chars split across HTTP chunks

### Root cause

At line 254, each incoming chunk is converted to `String` via `from_utf8_lossy`:
```rust
data: String::from_utf8_lossy(&bytes).into_owned(),
```

A valid UTF-8 multibyte character (e.g., `€` = 3 bytes) split across two HTTP
chunks produces U+FFFD in both chunk strings. The full-buffer validation at
line 277 passes (raw bytes valid when concatenated), so the cache write proceeds
with corrupted `chunks` data. Cache HITs replay corrupted content permanently.

### Strategy

Store raw `Bytes` + `offset_ms` during accumulation. After the full-buffer UTF-8
check passes, convert to `Vec<ResponseChunk>` using a carry-forward algorithm that
shifts incomplete trailing bytes to the next chunk. This preserves per-chunk timing
while guaranteeing valid UTF-8 in every chunk's `data` field.

### Changes

**1. Line 238 — change accumulator type:**

Before:
```rust
            let mut chunks: Vec<ResponseChunk> = Vec::new();
```
After:
```rust
            let mut chunks_raw: Vec<(u64, Bytes)> = Vec::new();
```

**2. Lines 250-256 — store raw bytes instead of converting:**

Before:
```rust
                        if do_cache {
                            let offset_ms = start.elapsed().as_millis() as u64;
                            response_buf.extend_from_slice(&bytes);
                            chunks.push(ResponseChunk {
                                offset_ms,
                                data: String::from_utf8_lossy(&bytes).into_owned(),
                            });
                        }
```
After:
```rust
                        if do_cache {
                            let offset_ms = start.elapsed().as_millis() as u64;
                            response_buf.extend_from_slice(&bytes);
                            chunks_raw.push((offset_ms, bytes.clone()));
                        }
```

(`bytes.clone()` on `Bytes` is an Arc refcount increment, no data copy.)

**3. Line 264 — update error log:**

Before:
```rust
                        tracing::warn!(err = %e, chunks = chunks.len(), "upstream stream error");
```
After:
```rust
                        tracing::warn!(err = %e, chunks = chunks_raw.len(), "upstream stream error");
```

**4. Line 286 — insert carry-forward conversion inside the cache-write guard:**

Insert between the `if stream_complete && do_cache && ...` line and the `let exchange = Exchange {` block:

```rust
                let chunks: Vec<ResponseChunk> = {
                    let mut carry: Vec<u8> = Vec::new();
                    chunks_raw
                        .into_iter()
                        .map(|(offset_ms, raw)| {
                            let mut buf = std::mem::take(&mut carry);
                            buf.extend_from_slice(&raw);
                            match std::str::from_utf8(&buf) {
                                Ok(s) => ResponseChunk {
                                    offset_ms,
                                    data: s.to_owned(),
                                },
                                Err(e) => {
                                    let valid_up_to = e.valid_up_to();
                                    let data = std::str::from_utf8(&buf[..valid_up_to])
                                        .expect("valid_up_to is a char boundary")
                                        .to_owned();
                                    carry = buf[valid_up_to..].to_vec();
                                    ResponseChunk { offset_ms, data }
                                }
                            }
                        })
                        .collect()
                };
```

The existing `chunks,` field in the `Exchange` struct literal at line 295 binds to
this new `let chunks` — no change needed there.

After the final chunk, `carry` is guaranteed empty because the full `response_buf`
passed the `from_utf8` check.

---

## Acceptance

```bash
cargo nextest run
cargo clippy --workspace --all-targets -- -D warnings
```

All 158 existing tests pass. The fix is invisible to tests that don't split multibyte
chars across chunk boundaries — it only changes behavior for that edge case.
