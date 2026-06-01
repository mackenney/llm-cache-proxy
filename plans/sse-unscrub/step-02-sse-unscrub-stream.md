# Step 02: SseUnscrubStream — Buffering State Machine

## Context

### Overall Objective

Implement SSE-aware unscrubbing so that fake keys distributed token-by-token across
Anthropic/OpenAI/Gemini SSE `data:` events are detected at the text level and replaced
before the response reaches the client or cache.

### Phase Context

Step 01 created the pure helper functions. This step adds the `SseUnscrubStream` type that
wraps a `ResponseStream`, buffers it completely, auto-detects SSE vs non-SSE, and applies
the appropriate unscrubbing strategy. Step 03 will wire this type into `ScrubExt`.

### This Step

Add `SseUnscrubStream` to `crates/lcp-server/src/ext/sse_unscrub.rs`. The type implements
`Stream<Item = Result<Bytes, io::Error>>` via a four-state machine. It buffers the full
upstream response in `Collecting`, then processes it in a boxed async `Processing` future
(either SSE text-level unscrub or raw-byte `unscrub_stream` for non-SSE), and drains the
result in `Emitting`. This buffering trade-off is acceptable: the proxy MUST correctly
restore secrets; streaming optimization is deferred to a follow-up.

## Prerequisites

- Step 01 merged (functions `is_sse_first_chunk`, `extract_text_field`, `set_text_field`
  are available in `sse_unscrub.rs`).

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_unscrub.rs` (output of step 01)
- `crates/lcp-server/src/ext/scrub.rs` — see how `unscrub_stream` is called; note the
  `io::Error::other(e.to_string())` pattern for mapping foreign errors
- `crates/lcp-server/src/extensions.rs` — `ResponseStream` type alias
- `crates/lcp-server/Cargo.toml` — confirm `futures-util` is available (it is)

## Implementation

### Task 1: Add imports to `sse_unscrub.rs`

Add at the top (after the existing `use` lines from step 01):

```rust
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{FutureExt, Stream, StreamExt};
use its_classified::types::{Entry, SessionKey};
use its_classified::unscrub_stream;

use crate::extensions::ResponseStream;
```

### Task 2: Define `SseUnscrubStream` and its state enum

```rust
/// Response stream wrapper that auto-detects SSE and applies the correct
/// unscrubbing strategy.
///
/// For SSE (`text/event-stream`): buffers all frames, accumulates provider text
/// fields, runs `unscrub_stream` on the concatenated text, redistributes the
/// restored text back into the original SSE frames, then drains the queue.
///
/// For non-SSE: buffers all bytes, runs `unscrub_stream` on the full buffer,
/// then drains.
///
/// Trade-off: complete buffering is required because a fake may span any number
/// of SSE events and its boundaries are unknown until the stream ends.
pub struct SseUnscrubStream {
    state: SseState,
}

enum SseState {
    /// Collecting all raw bytes from the inner stream.
    Collecting {
        inner: ResponseStream,
        raw_buf: Vec<u8>,
        is_sse: Option<bool>,   // None = first chunk not yet received
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    },
    /// Processing the fully-collected buffer (async).
    Processing(BoxFuture<'static, Result<VecDeque<Bytes>, io::Error>>),
    /// Draining the processed output queue.
    Emitting(VecDeque<Bytes>),
    /// Terminal: stream exhausted.
    Done,
}
```

### Task 3: Constructor

```rust
impl SseUnscrubStream {
    /// Wrap `stream` in SSE-aware unscrubbing. `provider` is used to locate the
    /// text field in SSE events.
    pub fn new(
        stream: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    ) -> Self {
        Self {
            state: SseState::Collecting {
                inner: stream,
                raw_buf: Vec::new(),
                is_sse: None,
                entries,
                session_key,
                provider,
            },
        }
    }
}
```

### Task 4: Implement `Stream` for `SseUnscrubStream`

```rust
impl Stream for SseUnscrubStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match &mut self.state {
                SseState::Collecting { inner, raw_buf, is_sse, .. } => {
                    // Poll inner in a loop to drain it fully before processing.
                    match Pin::new(inner).poll_next(cx) {
                        Poll::Ready(Some(Ok(chunk))) => {
                            if is_sse.is_none() {
                                *is_sse = Some(is_sse_first_chunk(&chunk));
                            }
                            raw_buf.extend_from_slice(&chunk);
                            // Continue looping — do not yield until inner is drained.
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            self.state = SseState::Done;
                            return Poll::Ready(Some(Err(e)));
                        }
                        Poll::Ready(None) => {
                            // Inner stream ended — start processing.
                            // Extract owned values from the Collecting state.
                            let (raw_buf, is_sse, entries, session_key, provider) =
                                match std::mem::replace(&mut self.state, SseState::Done) {
                                    SseState::Collecting {
                                        raw_buf,
                                        is_sse,
                                        entries,
                                        session_key,
                                        provider,
                                        ..
                                    } => (raw_buf, is_sse.unwrap_or(false), entries, session_key, provider),
                                    _ => unreachable!(),
                                };
                            let fut = process_buffer(raw_buf, entries, session_key, provider, is_sse).boxed();
                            self.state = SseState::Processing(fut);
                            continue;
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                SseState::Processing(fut) => {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(queue)) => {
                            self.state = SseState::Emitting(queue);
                            continue;
                        }
                        Poll::Ready(Err(e)) => {
                            self.state = SseState::Done;
                            return Poll::Ready(Some(Err(e)));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                SseState::Emitting(queue) => {
                    match queue.pop_front() {
                        Some(bytes) => return Poll::Ready(Some(Ok(bytes))),
                        None => {
                            self.state = SseState::Done;
                            return Poll::Ready(None);
                        }
                    }
                }
                SseState::Done => return Poll::Ready(None),
            }
        }
    }
}
```

### Task 5: `process_buffer` async function

This is the core logic. It must be `async` and `Send + 'static`.

```rust
async fn process_buffer(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
    is_sse: bool,
) -> Result<VecDeque<Bytes>, io::Error> {
    if !is_sse {
        // Non-SSE: run unscrub_stream on the raw bytes as a single-chunk in-memory stream.
        return unscrub_non_sse(raw, entries, session_key).await;
    }
    unscrub_sse(raw, entries, session_key, provider).await
}
```

### Task 6: `unscrub_non_sse` helper

```rust
async fn unscrub_non_sse(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
) -> Result<VecDeque<Bytes>, io::Error> {
    use futures_util::stream;

    let stream: ResponseStream = Box::pin(stream::once(async move {
        Ok::<Bytes, io::Error>(Bytes::from(raw))
    }));
    let us = unscrub_stream(stream, entries, session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let mut queue = VecDeque::new();
    futures_util::pin_mut!(us);
    while let Some(chunk) = us.next().await {
        let bytes = chunk.map_err(|e| io::Error::other(e.to_string()))?;
        queue.push_back(bytes);
    }
    Ok(queue)
}
```

### Task 7: `unscrub_sse` helper

This is the SSE text-level unscrub. It:
1. Splits raw bytes into SSE frames (split on `\n\n` boundaries)
2. For each frame, parses the `data: <json>` line
3. Extracts the provider text field from text-content events
4. Concatenates all extracted text into `text_buf`
5. Runs `unscrub_stream` on `text_buf` bytes → `restored_text`
6. Puts `restored_text` into the first text event; clears text in all other text events
7. Re-serializes modified text events; non-text events pass through unchanged
8. Returns all frames as `VecDeque<Bytes>`

```rust
async fn unscrub_sse(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
) -> Result<VecDeque<Bytes>, io::Error> {
    use futures_util::stream;

    // Step 1: Split into frames. SSE frames are separated by "\n\n".
    // Include the "\n\n" terminator in each frame for round-trip fidelity.
    let raw_str = String::from_utf8_lossy(&raw);
    let frames: Vec<&str> = raw_str.split_inclusive("\n\n").collect();

    // Step 2 & 3: Parse each frame. Extract text events.
    struct ParsedFrame {
        is_text: bool,
        json: Option<serde_json::Value>,    // parsed, with text field modified
        text: String,                        // original text content (empty for non-text)
        raw: String,                         // original frame string (for non-text pass-through)
    }

    let mut parsed: Vec<ParsedFrame> = Vec::with_capacity(frames.len());
    let mut text_buf = String::new();

    for frame in &frames {
        // Extract the "data: " line content.
        let data_content = frame
            .lines()
            .find_map(|l| l.strip_prefix("data: "));

        let Some(data_str) = data_content else {
            // No data line (e.g., comment-only frame or keep-alive). Pass through.
            parsed.push(ParsedFrame {
                is_text: false, json: None, text: String::new(), raw: frame.to_string(),
            });
            continue;
        };

        match serde_json::from_str::<serde_json::Value>(data_str) {
            Err(_) => {
                // Not JSON (e.g., "data: [DONE]"). Pass through.
                parsed.push(ParsedFrame {
                    is_text: false, json: None, text: String::new(), raw: frame.to_string(),
                });
            }
            Ok(json) => {
                let extracted = extract_text_field(&json, provider)
                    .map(str::to_owned);
                let is_text = extracted.is_some();
                let text = extracted.unwrap_or_default();
                text_buf.push_str(&text);
                parsed.push(ParsedFrame {
                    is_text, json: Some(json), text, raw: frame.to_string(),
                });
            }
        }
    }

    // Step 4 & 5: If no text events, nothing to unscrub — pass frames through.
    if text_buf.is_empty() {
        let mut queue = VecDeque::new();
        for f in parsed {
            queue.push_back(Bytes::from(f.raw.into_bytes()));
        }
        return Ok(queue);
    }

    // Run unscrub_stream on the concatenated text buffer.
    let text_bytes = Bytes::from(text_buf.as_bytes().to_vec());
    let text_stream: ResponseStream = Box::pin(stream::once(async move {
        Ok::<Bytes, io::Error>(text_bytes)
    }));
    let us = unscrub_stream(text_stream, entries, session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    futures_util::pin_mut!(us);
    let mut restored_bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = us.next().await {
        restored_bytes.extend_from_slice(&chunk.map_err(|e| io::Error::other(e.to_string()))?);
    }
    let restored_text = String::from_utf8_lossy(&restored_bytes).into_owned();

    // Step 6: Redistribute restored text. Strategy: first text event gets all
    // restored text; subsequent text events get empty string.
    // This preserves all content while changing event granularity (acceptable).
    let mut first_text_done = false;
    let mut queue = VecDeque::new();

    for mut frame in parsed {
        if !frame.is_text {
            queue.push_back(Bytes::from(frame.raw.into_bytes()));
            continue;
        }
        let Some(mut json) = frame.json.take() else {
            queue.push_back(Bytes::from(frame.raw.into_bytes()));
            continue;
        };
        let new_text = if !first_text_done {
            first_text_done = true;
            restored_text.clone()
        } else {
            String::new()
        };
        set_text_field(&mut json, provider, new_text);
        // Re-serialize: rebuild the frame as "data: <json>\n\n".
        // Other lines in the frame (e.g., "event:", ":") are preserved by
        // reconstructing only the data line; in practice provider SSE frames
        // contain exactly one line ("data: <json>") so this is equivalent.
        let reconstructed = format!(
            "data: {}\n\n",
            serde_json::to_string(&json).map_err(|e| io::Error::other(e.to_string()))?
        );
        queue.push_back(Bytes::from(reconstructed.into_bytes()));
    }

    Ok(queue)
}
```

**Important note on multi-line frames**: The code above uses `split_inclusive("\n\n")` and then
only reconstructs the `data:` line for text events. If a provider frame contains other lines
(e.g., `id: ...` or `event: ...`), those are lost. For all four supported providers, each SSE
frame is a single `data: <json>\n\n` line with no other lines, so this is correct. If a frame
has no text field, it is passed through via `frame.raw` (the original bytes), so no data loss.

### Task 8: Unit tests

Add to the `#[cfg(test)]` module in `sse_unscrub.rs`:

```rust
#[tokio::test]
async fn sse_unscrub_stream_passthrough_no_secrets() {
    // SSE stream with no fakes — output bytes must equal input bytes.
    use futures_util::stream;
    let input = b"data: {\"type\":\"message_start\"}\n\ndata: {\"type\":\"message_stop\"}\n\n";
    let stream: ResponseStream = Box::pin(stream::once(async move {
        Ok::<Bytes, io::Error>(Bytes::from_static(input))
    }));
    // Use empty entries + dummy session key → no unscrubbing applied.
    let session_key = SessionKey::from_bytes([0u8; 32]);
    let us = SseUnscrubStream::new(stream, vec![], session_key, Provider::Anthropic);
    let out: Vec<Bytes> = futures_util::StreamExt::collect::<Vec<_>>(us)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    let out_bytes: Vec<u8> = out.iter().flat_map(|b| b.to_vec()).collect();
    assert_eq!(out_bytes, input);
}

#[tokio::test]
async fn non_sse_passthrough_no_secrets() {
    use futures_util::stream;
    let input = b"{\"result\":\"ok\"}";
    let stream: ResponseStream = Box::pin(stream::once(async move {
        Ok::<Bytes, io::Error>(Bytes::from_static(input))
    }));
    let session_key = SessionKey::from_bytes([0u8; 32]);
    let us = SseUnscrubStream::new(stream, vec![], session_key, Provider::OpenAi);
    let out: Vec<u8> = futures_util::StreamExt::collect::<Vec<_>>(us)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .flat_map(|b| b.to_vec())
        .collect();
    assert_eq!(out, input);
}

#[tokio::test]
async fn empty_stream_produces_empty_output() {
    use futures_util::stream;
    let stream: ResponseStream = Box::pin(stream::empty());
    let session_key = SessionKey::from_bytes([0u8; 32]);
    let us = SseUnscrubStream::new(stream, vec![], session_key, Provider::Gemini);
    let out: Vec<_> = futures_util::StreamExt::collect::<Vec<_>>(us).await;
    assert!(out.is_empty());
}
```

The full fake-replacement test requires `its_classified::scrub` and belongs in the integration
tests in step 03.

## Acceptance Criteria

- [ ] `cargo build -p lcp-server` exits 0
- [ ] `cargo nextest run -p lcp-server --lib 2>&1 | grep -E 'FAILED|error\[|^test result'` shows no failures and `test result: ok`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `grep -c 'pub struct SseUnscrubStream' crates/lcp-server/src/ext/sse_unscrub.rs` outputs `1`
- [ ] `grep -c 'async fn process_buffer' crates/lcp-server/src/ext/sse_unscrub.rs` outputs `1`
- [ ] `cargo nextest run -p lcp-server --lib sse_unscrub 2>&1 | grep PASSED | wc -l` shows at least 13 passing tests (10 from step 01 + 3 new)

## Reviewer Instructions

```bash
cd /home/ignacio/pr/llm-cache-proxy

cargo build -p lcp-server 2>&1 | tail -3; echo "build exit: $?"

cargo nextest run -p lcp-server --lib 2>&1 | grep -E 'FAILED|^test result'

cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep -E 'error|warning.*unused' | head -10

# Verify new types exist
grep -n 'pub struct SseUnscrubStream\|async fn process_buffer\|async fn unscrub_sse\|async fn unscrub_non_sse' \
    crates/lcp-server/src/ext/sse_unscrub.rs
```

Expected: build exits 0, no test failures, clippy clean, all four grep hits present.

## Rollback

`git revert HEAD` or `git checkout -- crates/lcp-server/src/ext/sse_unscrub.rs` to restore
the state from step 01.
