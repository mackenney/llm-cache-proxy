//! SSE-aware unscrubbing helpers for `ScrubExt`.
//!
//! These functions are the building blocks for semantic-level secret
//! restoration in `text/event-stream` responses, where raw-byte Aho-Corasick
//! fails because the fake key never appears contiguously in the byte stream.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{FutureExt, Stream, StreamExt};
use its_classified::types::{Entry, SessionKey};
use its_classified::unscrub_stream;
use lcp_core::Provider;
use serde_json::Value;

use crate::extensions::ResponseStream;

/// Returns `true` if the first bytes of a response chunk look like an SSE stream.
///
/// Detects both `data: ` and `event: ` line starters. Providers that emit only
/// `data:` lines (OpenAI, OpenRouter, Gemini) start with `data: `; Anthropic's
/// real API prefixes each data line with a named `event:` line, so the stream
/// starts with `event: ` instead. Non-SSE JSON responses begin with `{`.
pub fn is_sse_first_chunk(bytes: &[u8]) -> bool {
    bytes.starts_with(b"data: ") || bytes.starts_with(b"event: ")
}

/// Returns the provider-specific text field from a parsed SSE event JSON, if present.
///
/// Returns `None` for events that do not carry text content (non-text events or
/// when the relevant fields are absent).
pub fn extract_text_field(json: &Value, provider: Provider) -> Option<&str> {
    match provider {
        Provider::Anthropic => {
            if json["type"].as_str()? != "content_block_delta" {
                return None;
            }
            if json["delta"]["type"].as_str()? != "text_delta" {
                return None;
            }
            json["delta"]["text"].as_str()
        }
        Provider::OpenAi | Provider::OpenRouter => {
            json.pointer("/choices/0/delta/content")?.as_str()
        }
        Provider::Gemini => json.pointer("/candidates/0/content/parts/0/text")?.as_str(),
    }
}

/// Sets the provider-specific text field in a mutable SSE event JSON.
///
/// Returns `true` if the field was located and set; `false` if not found
/// (non-text event).
pub fn set_text_field(json: &mut Value, provider: Provider, text: String) -> bool {
    match provider {
        Provider::Anthropic => {
            if let Some(v) = json.get_mut("delta").and_then(|d| d.get_mut("text")) {
                *v = Value::String(text);
                true
            } else {
                false
            }
        }
        Provider::OpenAi | Provider::OpenRouter => {
            if let Some(v) = json
                .get_mut("choices")
                .and_then(|c| c.get_mut(0))
                .and_then(|c| c.get_mut("delta"))
                .and_then(|d| d.get_mut("content"))
            {
                *v = Value::String(text);
                true
            } else {
                false
            }
        }
        Provider::Gemini => {
            if let Some(v) = json
                .get_mut("candidates")
                .and_then(|c| c.get_mut(0))
                .and_then(|c| c.get_mut("content"))
                .and_then(|c| c.get_mut("parts"))
                .and_then(|p| p.get_mut(0))
                .and_then(|p| p.get_mut("text"))
            {
                *v = Value::String(text);
                true
            } else {
                false
            }
        }
    }
}

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
        is_sse: Option<bool>,
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

impl Stream for SseUnscrubStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match std::mem::replace(&mut self.state, SseState::Done) {
                SseState::Collecting {
                    mut inner,
                    mut raw_buf,
                    mut is_sse,
                    entries,
                    session_key,
                    provider,
                } => {
                    match inner.as_mut().poll_next(cx) {
                        Poll::Ready(Some(Ok(chunk))) => {
                            if is_sse.is_none() && !chunk.is_empty() {
                                is_sse = Some(is_sse_first_chunk(&chunk));
                            }
                            raw_buf.extend_from_slice(&chunk);
                            // Restore state and loop — drain inner before processing.
                            self.state = SseState::Collecting {
                                inner,
                                raw_buf,
                                is_sse,
                                entries,
                                session_key,
                                provider,
                            };
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            // state is already Done
                            return Poll::Ready(Some(Err(e)));
                        }
                        Poll::Ready(None) => {
                            let is_sse_flag = is_sse.unwrap_or(false);
                            let fut = process_buffer(
                                raw_buf,
                                entries,
                                session_key,
                                provider,
                                is_sse_flag,
                            )
                            .boxed();
                            self.state = SseState::Processing(fut);
                            continue;
                        }
                        Poll::Pending => {
                            // Restore state — more data coming.
                            self.state = SseState::Collecting {
                                inner,
                                raw_buf,
                                is_sse,
                                entries,
                                session_key,
                                provider,
                            };
                            return Poll::Pending;
                        }
                    }
                }
                SseState::Processing(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(queue)) => {
                        self.state = SseState::Emitting(queue);
                        continue;
                    }
                    Poll::Ready(Err(e)) => {
                        // state is already Done
                        return Poll::Ready(Some(Err(e)));
                    }
                    Poll::Pending => {
                        self.state = SseState::Processing(fut);
                        return Poll::Pending;
                    }
                },
                SseState::Emitting(mut queue) => match queue.pop_front() {
                    Some(bytes) => {
                        if !queue.is_empty() {
                            self.state = SseState::Emitting(queue);
                        }
                        return Poll::Ready(Some(Ok(bytes)));
                    }
                    None => return Poll::Ready(None),
                },
                SseState::Done => return Poll::Ready(None),
            }
        }
    }
}

async fn process_buffer(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
    is_sse: bool,
) -> Result<VecDeque<Bytes>, io::Error> {
    // Nothing to process for an empty buffer.
    if raw.is_empty() {
        return Ok(VecDeque::new());
    }
    if !is_sse {
        // Non-SSE: run unscrub_stream on the raw bytes as a single-chunk in-memory stream.
        return unscrub_non_sse(raw, entries, session_key).await;
    }
    unscrub_sse(raw, entries, session_key, provider).await
}

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

async fn unscrub_sse(
    raw: Vec<u8>,
    entries: Vec<Entry>,
    session_key: SessionKey,
    provider: Provider,
) -> Result<VecDeque<Bytes>, io::Error> {
    use futures_util::stream;

    // Step 1: Split into frames. SSE frames are separated by "\n\n".
    // Include the "\n\n" terminator in each frame for round-trip fidelity.
    let raw_str = String::from_utf8(raw)
        .map_err(|e| io::Error::other(format!("SSE response contained non-UTF8 bytes: {e}")))?;
    // Normalize \r\n → \n before splitting so both line-ending styles are handled.
    // The WHATWG EventSource spec permits \r, \n, or \r\n line endings; providers
    // that use \r\n\r\n frame separators would otherwise produce no split boundaries.
    let normalized = raw_str.replace("\r\n", "\n");
    let frames: Vec<&str> = normalized.split_inclusive("\n\n").collect();

    // Step 2 & 3: Parse each frame and extract text content.
    struct ParsedFrame {
        is_text: bool,
        json: Option<serde_json::Value>,
        raw: String,
    }

    let mut parsed: Vec<ParsedFrame> = Vec::with_capacity(frames.len());
    let mut text_buf = String::new();

    for frame in &frames {
        let data_content = frame.lines().find_map(|l| l.strip_prefix("data: "));

        let Some(data_str) = data_content else {
            // No data line (comment-only frame or keep-alive). Pass through.
            parsed.push(ParsedFrame {
                is_text: false,
                json: None,
                raw: frame.to_string(),
            });
            continue;
        };

        match serde_json::from_str::<serde_json::Value>(data_str) {
            Err(_) => {
                // Not JSON (e.g., "data: [DONE]"). Pass through.
                parsed.push(ParsedFrame {
                    is_text: false,
                    json: None,
                    raw: frame.to_string(),
                });
            }
            Ok(json) => {
                let extracted = extract_text_field(&json, provider).map(str::to_owned);
                let is_text = extracted.is_some();
                if let Some(ref t) = extracted {
                    text_buf.push_str(t);
                }
                parsed.push(ParsedFrame {
                    is_text,
                    json: Some(json),
                    raw: frame.to_string(),
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
    let text_stream: ResponseStream =
        Box::pin(stream::once(
            async move { Ok::<Bytes, io::Error>(text_bytes) },
        ));
    let us = unscrub_stream(text_stream, entries, session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    futures_util::pin_mut!(us);
    let mut restored_bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = us.next().await {
        restored_bytes.extend_from_slice(&chunk.map_err(|e| io::Error::other(e.to_string()))?);
    }
    let mut restored_text = String::from_utf8(restored_bytes)
        .map_err(|e| io::Error::other(format!("unscrub_stream produced non-UTF8 bytes: {e}")))?;

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
            std::mem::take(&mut restored_text)
        } else {
            String::new()
        };
        if !set_text_field(&mut json, provider, new_text) {
            // This branch is unreachable if extract_text_field and set_text_field
            // are in sync, but the explicit guard surfaces future divergence early.
            return Err(io::Error::other(format!(
                "set_text_field failed for provider {provider:?} on frame that extract_text_field accepted"
            )));
        }
        // Re-serialize: rebuild the frame preserving non-data lines (event:, id:,
        // retry:) from the original, then append the updated data line.
        let prefix_lines: String = frame
            .raw
            .lines()
            .filter(|l| !l.starts_with("data:") && !l.is_empty())
            .map(|l| format!("{l}\n"))
            .collect();
        let reconstructed = format!(
            "{}data: {}\n\n",
            prefix_lines,
            serde_json::to_string(&json).map_err(|e| io::Error::other(e.to_string()))?
        );
        queue.push_back(Bytes::from(reconstructed.into_bytes()));
    }

    Ok(queue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_sse_detects_data_prefix() {
        assert!(is_sse_first_chunk(
            b"data: {\"type\":\"message_start\"}\n\n"
        ));
    }

    #[test]
    fn is_sse_detects_event_prefix() {
        // Real Anthropic SSE streams begin with an `event:` line before `data:`.
        assert!(is_sse_first_chunk(
            b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n"
        ));
    }

    #[test]
    fn is_sse_rejects_json() {
        assert!(!is_sse_first_chunk(b"{\"type\":\"message\"}"));
    }

    #[test]
    fn is_sse_rejects_empty() {
        assert!(!is_sse_first_chunk(b""));
    }

    #[test]
    fn is_sse_detection_skips_empty_first_chunk() {
        // Regression: an empty leading chunk must not latch is_sse to false.
        // The detection should wait for a non-empty chunk.
        // This test documents the intent; the actual guard is in SseUnscrubStream::poll_next.
        assert!(!is_sse_first_chunk(b""));
        assert!(is_sse_first_chunk(b"data: "));
        assert!(is_sse_first_chunk(b"event: "));
    }

    #[test]
    fn extract_anthropic_text_delta() {
        let v = json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "hello" }
        });
        assert_eq!(extract_text_field(&v, Provider::Anthropic), Some("hello"));
    }

    #[test]
    fn extract_anthropic_skips_message_start() {
        let v = json!({ "type": "message_start" });
        assert_eq!(extract_text_field(&v, Provider::Anthropic), None);
    }

    #[test]
    fn extract_anthropic_skips_content_block_stop() {
        let v = json!({ "type": "content_block_stop", "index": 0 });
        assert_eq!(extract_text_field(&v, Provider::Anthropic), None);
    }

    #[test]
    fn extract_openai_delta_content() {
        let v = json!({ "choices": [{ "delta": { "content": "hi" } }] });
        assert_eq!(extract_text_field(&v, Provider::OpenAi), Some("hi"));
    }

    #[test]
    fn extract_openai_skips_null_content() {
        let v = json!({ "choices": [{ "delta": {} }] });
        assert_eq!(extract_text_field(&v, Provider::OpenAi), None);
    }

    #[test]
    fn extract_gemini_text() {
        let v = json!({
            "candidates": [{ "content": { "parts": [{ "text": "hi" }] } }]
        });
        assert_eq!(extract_text_field(&v, Provider::Gemini), Some("hi"));
    }

    #[test]
    fn extract_gemini_skips_empty_parts() {
        let v = json!({
            "candidates": [{ "content": { "parts": [] } }]
        });
        assert_eq!(extract_text_field(&v, Provider::Gemini), None);
    }

    #[test]
    fn set_anthropic_text_field_replaces() {
        let mut v = json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "old" }
        });
        assert!(set_text_field(
            &mut v,
            Provider::Anthropic,
            "new".to_owned()
        ));
        assert_eq!(v["delta"]["text"], json!("new"));
    }

    #[test]
    fn set_anthropic_returns_false_for_non_text() {
        let mut v = json!({ "type": "message_start" });
        assert!(!set_text_field(&mut v, Provider::Anthropic, "x".to_owned()));
    }

    #[test]
    fn set_openai_text_field_replaces() {
        let mut v = json!({ "choices": [{ "delta": { "content": "old" } }] });
        assert!(set_text_field(&mut v, Provider::OpenAi, "new".to_owned()));
        assert_eq!(v["choices"][0]["delta"]["content"], json!("new"));
    }

    #[test]
    fn set_gemini_text_field_replaces() {
        let mut v = json!({
            "candidates": [{ "content": { "parts": [{ "text": "old" }] } }]
        });
        assert!(set_text_field(&mut v, Provider::Gemini, "new".to_owned()));
        assert_eq!(
            v["candidates"][0]["content"]["parts"][0]["text"],
            json!("new")
        );
    }

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

    #[tokio::test]
    async fn sse_event_lines_preserved_through_text_frame_reconstruction() {
        // Text frames that have an `event:` prefix line must have that line
        // preserved in the output — reconstruction must not drop non-data fields.
        use futures_util::stream;
        let input = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let stream: ResponseStream = Box::pin(stream::once(async move {
            Ok::<Bytes, io::Error>(Bytes::from(input.as_bytes().to_vec()))
        }));
        let session_key = SessionKey::from_bytes([0u8; 32]);
        let us = SseUnscrubStream::new(stream, vec![], session_key, Provider::Anthropic);
        let out: Vec<u8> = futures_util::StreamExt::collect::<Vec<_>>(us)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .flat_map(|b| b.to_vec())
            .collect();
        let out_str = String::from_utf8(out).unwrap();
        // Each event: + data: pair must be in the same SSE frame (no \n\n between them).
        let frames: Vec<&str> = out_str.split("\n\n").filter(|f| !f.is_empty()).collect();
        let text_frame = frames
            .iter()
            .find(|f| f.contains("content_block_delta"))
            .expect("must have a content_block_delta frame");
        assert!(
            text_frame.contains("event: content_block_delta\n") && text_frame.contains("data: "),
            "event: and data: must be in the same frame; got: {text_frame:?}"
        );
        let stop_frame = frames
            .iter()
            .find(|f| f.contains("message_stop"))
            .expect("must have a message_stop frame");
        assert!(
            stop_frame.contains("event: message_stop\n") && stop_frame.contains("data: "),
            "event: and data: must be in the same frame; got: {stop_frame:?}"
        );
    }
}
