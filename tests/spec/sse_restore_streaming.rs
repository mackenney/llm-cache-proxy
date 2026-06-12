//! Spec invariant tests for §SSE-Aware Restore: Sliding Window (SPEC.md).
//!
//! These tests enforce that `SseRestoreStream` emits output before EOF
//! (per-FieldKey sliding window), rather than buffering the entire stream.
//!
//! All tests pass against the sliding-window implementation and serve as
//! regression guards.
//!
//! Tests 1 and 2 (emits-before-eof, passthrough) would time out if
//! full-buffering regressed.
//!
//! Test 3 (`correctness_after_window`) covers the fake sent as bare text.
//!
//! Tests 4 and 5 cover the embedded-JSON case: a fake key wrapped in JSON
//! (e.g. `{"key": "FAKE_KEY"}`) whose total length exceeds max_fake_len.
//! Before the fix, flush_safe_prefix would split the fake across two restore
//! calls, preventing Aho-Corasick from matching the complete pattern.
use std::io;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use lcp_core::Provider;
use lcp_server::ResponseStream;
use lcp_server::ext::sse_restore::SseRestoreStream;

/// Build a complete Anthropic `content_block_delta` / `text_delta` SSE frame.
fn anthropic_text_delta_frame(index: u32, text: &str) -> Bytes {
    let json = serde_json::json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {
            "type": "text_delta",
            "text": text
        }
    });
    Bytes::from(format!(
        "event: content_block_delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

/// Build an Anthropic `content_block_delta` / `input_json_delta` SSE frame.
fn anthropic_input_json_delta_frame(index: u32, partial_json: &str) -> Bytes {
    let json = serde_json::json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {
            "type": "input_json_delta",
            "partial_json": partial_json
        }
    });
    Bytes::from(format!(
        "event: content_block_delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

/// Build an OpenAI `tool_calls.function.arguments` delta SSE frame.
fn openai_tool_call_args_frame(tool_index: u64, arguments: &str) -> Bytes {
    let json = serde_json::json!({
        "choices": [{"delta": {
            "tool_calls": [{"index": tool_index, "function": {"arguments": arguments}}]
        }}]
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

/// Build an OpenAI `[DONE]` terminator frame.
fn openai_done_frame() -> Bytes {
    Bytes::from_static(b"data: [DONE]\n\n")
}
/// Wrap an unbounded `mpsc::UnboundedReceiver` as a `ResponseStream`.
///
/// Using an unbounded channel avoids send-side blocking; sends are sync and
/// complete immediately regardless of how many frames have been queued.
/// The stream ends when the sender is dropped.
fn unbounded_receiver_to_stream(
    rx: tokio::sync::mpsc::UnboundedReceiver<Result<Bytes, io::Error>>,
) -> ResponseStream {
    Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

/// INV-SSE-WINDOW-1: SseRestoreStream MUST emit output before the source stream
/// reaches EOF once the sliding window is full.
///
/// The full-buffer approach produced no output until the sender was dropped
/// (EOF). This test sends `max_fake_len + 5` frames without closing the sender,
/// then polls for output within a 2-second timeout.
/// Full-buffer → timeout → assertion fails. Sliding-window → PASSES.
#[tokio::test]
async fn inv_sse_streaming_emits_before_eof() {
    // NOT a real credential — synthetic Anthropic-format key matching the
    // structural pattern so doppel::patterns::all() detects it.
    let secret =
        b"sk-ant-api03-w8bVJRHra9S96i3ios_XhbLgzEBjS6qjPUEgiPrWjN2OeICCY1lwhK3Z35Z_jM89STjqSOxHh6GWGkG2R7uv-AohQLmK9AA";

    let result = doppel::swap(secret, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result
        .entries
        .iter()
        .map(|e| e.fake.len())
        .max()
        .unwrap_or(0);
    assert!(max_fake_len > 0, "fake must have non-zero length");

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let mut restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Anthropic,
    );

    // Send more than max_fake_len single-char frames WITHOUT closing the sender.
    // Unbounded sends are non-blocking — no deadlock risk.
    // A sliding-window implementation can start emitting after max_fake_len chars
    // have accumulated; a full-buffer approach would wait for EOF instead.
    for _ in 0..(max_fake_len + 5) {
        tx.send(Ok(anthropic_text_delta_frame(0, "a"))).unwrap();
    }

    // Poll for output. Sliding-window: output arrives → Ok. Full-buffer: no
    // output until EOF → timeout → Err → assertion fails → test FAILS.
    let got = tokio::time::timeout(Duration::from_secs(2), restore.next()).await;

    assert!(
        got.is_ok(),
        "SseRestoreStream MUST emit output before EOF once the window is full; \
         full-buffer behavior detected (2-second timeout hit with {} frames in flight)",
        max_fake_len + 5
    );
    let item = got.unwrap();
    assert!(item.is_some(), "expected Some(_) from stream");
    assert!(item.unwrap().is_ok(), "expected Ok(_) from stream");

    // Keep sender alive to confirm the assertion above was not due to EOF.
    drop(tx);
}

/// INV-SSE-WINDOW-2: With no entries, every frame must pass through immediately
/// (window size = 0, no accumulation required).
///
/// A full-buffer approach would buffer even with no entries → timeout → FAILS.
/// Sliding-window: immediate passthrough → PASSES.
#[tokio::test]
async fn inv_sse_streaming_no_entries_passthrough_immediate() {
    let session_key = doppel::SessionKey::from_bytes([0u8; 32]);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let mut restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        vec![],
        session_key,
        Provider::Anthropic,
    );

    for i in 0..3u32 {
        // Unbounded send is non-blocking; the frame is in the channel
        // before we poll.
        tx.send(Ok(anthropic_text_delta_frame(0, "a"))).unwrap();

        let got = tokio::time::timeout(Duration::from_secs(1), restore.next()).await;
        assert!(
            got.is_ok(),
            "frame {i}: SseRestoreStream with empty entries MUST emit immediately; \
             full-buffer behavior detected (1-second timeout hit)"
        );
        let item = got.unwrap();
        assert!(item.is_some(), "frame {i}: expected Some(_) from stream");
        assert!(
            item.unwrap().is_ok(),
            "frame {i}: expected Ok(_) from stream"
        );
    }

    drop(tx);
}

/// INV-SSE-WINDOW-3: Correctness regression guard — restored output must
/// contain the original secret and must not contain the fake.
///
/// Passes on both old and new implementations. Ensures the sliding-window
/// implementation does not break restoration correctness.
#[tokio::test]
async fn inv_sse_streaming_correctness_after_window() {
    // NOT a real credential — synthetic Anthropic-format key.
    let secret =
        b"sk-ant-api03-w8bVJRHra9S96i3ios_XhbLgzEBjS6qjPUEgiPrWjN2OeICCY1lwhK3Z35Z_jM89STjqSOxHh6GWGkG2R7uv-AohQLmK9AA";

    let result = doppel::swap(secret, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );

    let fake_bytes = result.entries[0].fake.clone();
    let fake_str = String::from_utf8(fake_bytes).expect("fake must be UTF-8 for this test");

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let mut restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Anthropic,
    );

    // Send the fake key character by character.
    for ch in fake_str.chars() {
        tx.send(Ok(anthropic_text_delta_frame(0, &ch.to_string())))
            .unwrap();
    }
    // Signal EOF.
    drop(tx);

    // Collect all output.
    let mut all_bytes = Vec::new();
    while let Some(item) = restore.next().await {
        let chunk = item.expect("unexpected error in restore stream");
        all_bytes.extend_from_slice(&chunk);
    }

    // Parse all text delta values from the output SSE frames.
    let output_str = String::from_utf8(all_bytes).expect("output must be valid UTF-8");
    let mut text_content = String::new();
    for line in output_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(text) = json["delta"]["text"].as_str() {
                    text_content.push_str(text);
                }
            }
        }
    }

    let secret_str = std::str::from_utf8(secret).unwrap();
    assert!(
        text_content.contains(secret_str),
        "restored output must contain the original secret; concatenated delta text: {text_content:?}"
    );
    assert!(
        !text_content.contains(fake_str.as_str()),
        "restored output must not contain the fake; concatenated delta text: {text_content:?}"
    );
}

/// INV-SSE-WINDOW-4: A fake key embedded inside a JSON string (as in Anthropic
/// `input_json_delta` tool-call args) MUST be restored even when the surrounding
/// JSON characters push the accumulator past `max_fake_len`.
///
/// Before the fix, `flush_safe_prefix` would flush the safe prefix when
/// `accum.len() > max_fake_len`. If the safe prefix ended with the first few
/// bytes of the fake (e.g. `{"key": "sk`), those bytes were passed through
/// `doppel_restore` unchanged (no complete pattern match). The remaining
/// accumulator then started mid-fake, which also never matched. Result: the
/// original secret was absent from the output and the fake was present.
#[tokio::test]
async fn inv_sse_streaming_input_json_delta_embedded_key_restores_secret() {
    // NOT a real credential — synthetic Anthropic-format key.
    let secret =
        b"sk-ant-api03-w8bVJRHra9S96i3ios_XhbLgzEBjS6qjPUEgiPrWjN2OeICCY1lwhK3Z35Z_jM89STjqSOxHh6GWGkG2R7uv-AohQLmK9AA";

    let result = doppel::swap(secret, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );

    let fake_bytes = result.entries[0].fake.clone();
    let fake_str = String::from_utf8(fake_bytes).expect("fake must be UTF-8 for this test");

    // Simulate the partial_json content the model echoes back: the tool-call input
    // JSON embeds the fake as a string value. The 11 surrounding JSON bytes push
    // the accumulator past max_fake_len, triggering flush_safe_prefix mid-fake.
    let full_json = format!("{{\"key\": \"{}\"}}", fake_str);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let mut restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Anthropic,
    );

    // Send the full JSON as a single input_json_delta frame so the accumulator
    // immediately exceeds max_fake_len and triggers flush_safe_prefix.
    tx.send(Ok(anthropic_input_json_delta_frame(1, &full_json)))
        .unwrap();
    drop(tx);

    let mut all_bytes = Vec::new();
    while let Some(item) = restore.next().await {
        let chunk = item.expect("unexpected error in restore stream");
        all_bytes.extend_from_slice(&chunk);
    }

    // Concatenate all partial_json values from the synthetic output frames.
    let output_str = String::from_utf8(all_bytes).expect("output must be valid UTF-8");
    let mut json_content = String::new();
    for line in output_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(pj) = json["delta"]["partial_json"].as_str() {
                    json_content.push_str(pj);
                }
            }
        }
    }

    let secret_str = std::str::from_utf8(secret).unwrap();
    assert!(
        json_content.contains(secret_str),
        "restored output must contain the original secret in partial_json; got: {json_content:?}"
    );
    assert!(
        !json_content.contains(fake_str.as_str()),
        "restored output must not contain the fake; got: {json_content:?}"
    );
}

/// INV-SSE-WINDOW-5: Same embedded-JSON issue for OpenAI `tool_calls.function.arguments`.
///
/// OpenAI streams tool-call arguments incrementally; the JSON wrapper
/// (`{"key": "..."`) again pushes the accumulator past max_fake_len.
#[tokio::test]
async fn inv_sse_streaming_openai_tool_call_embedded_key_restores_secret() {
    // NOT a real credential — synthetic OpenAI-classic-format key.
    let secret = b"sk-v0zsmdzWwRZktfsJIdQWQvKdIYk1LYrtuF3hWeJep2YvHzQ3";

    let result = doppel::swap(secret, &[doppel::patterns::openai_classic()]).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic OpenAI key"
    );

    let fake_bytes = result.entries[0].fake.clone();
    let fake_str = String::from_utf8(fake_bytes).expect("fake must be UTF-8 for this test");

    let full_json = format!("{{\"key\": \"{}\"}}", fake_str);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let mut restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenAi,
    );

    tx.send(Ok(openai_tool_call_args_frame(0, &full_json)))
        .unwrap();
    tx.send(Ok(openai_done_frame())).unwrap();
    drop(tx);

    let mut all_bytes = Vec::new();
    while let Some(item) = restore.next().await {
        let chunk = item.expect("unexpected error in restore stream");
        all_bytes.extend_from_slice(&chunk);
    }

    let output_str = String::from_utf8(all_bytes).expect("output must be valid UTF-8");
    let mut args_content = String::new();
    for line in output_str.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(args) =
                    json["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str()
                {
                    args_content.push_str(args);
                }
            }
        }
    }

    let secret_str = std::str::from_utf8(secret).unwrap();
    assert!(
        args_content.contains(secret_str),
        "restored output must contain the original secret in tool_calls arguments; got: {args_content:?}"
    );
    assert!(
        !args_content.contains(fake_str.as_str()),
        "restored output must not contain the fake; got: {args_content:?}"
    );
}
