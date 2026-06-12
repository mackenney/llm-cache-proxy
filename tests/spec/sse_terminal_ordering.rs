//! Spec invariant tests for §Terminal Event Ordering (SPEC.md VC-SSE-14..20).
//!
//! Each test encodes one MUST clause from `crates/lcp-server/SPEC.md §Terminal
//! Event Ordering` as an external behavioral contract. They serve as regression
//! guards: reverting the terminal-event flush in `process_one_frame` causes at least
//! `vc_sse_14_anthropic_block_stop_ordering` and
//! `vc_sse_17_openai_finish_reason_ordering` to FAIL.
//!
//! Critical sizing rule: for every held-content scenario the accumulated text
//! fed before the terminal totals EXACTLY `max_fake_len` bytes so no
//! safe-prefix flush fires early and the entire secret appears in a well-defined
//! position relative to the terminal frame.

use std::io;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use lcp_core::Provider;
use lcp_server::ResponseStream;
use lcp_server::ext::sse_restore::SseRestoreStream;

// NOT a real credential — synthetic Anthropic-format key matching the
// structural pattern so doppel::patterns::all() detects it.
const SECRET: &[u8] =
    b"sk-ant-api03-w8bVJRHra9S96i3ios_XhbLgzEBjS6qjPUEgiPrWjN2OeICCY1lwhK3Z35Z_jM89STjqSOxHh6GWGkG2R7uv-AohQLmK9AA";

fn unbounded_receiver_to_stream(
    rx: tokio::sync::mpsc::UnboundedReceiver<Result<Bytes, io::Error>>,
) -> ResponseStream {
    Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

/// Returns the byte offset of `needle` in `haystack`, panicking with the needle
/// if not found.
fn pos(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("needle {:?} not found in output:\n{haystack}", needle))
}

/// Drain `restore` to EOF and return all output as a UTF-8 string.
/// The test fails if the stream does not complete within 5 seconds.
async fn collect_all(restore: SseRestoreStream) -> String {
    let fut = async move {
        let mut restore = restore;
        let mut all_bytes = Vec::new();
        while let Some(item) = restore.next().await {
            let chunk = item.expect("unexpected error in restore stream");
            all_bytes.extend_from_slice(&chunk);
        }
        String::from_utf8_lossy(&all_bytes).into_owned()
    };
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .expect("SseRestoreStream did not complete within 5 seconds")
}

fn ant_text_delta(index: u32, text: &str) -> Bytes {
    let json = serde_json::json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {"type": "text_delta", "text": text}
    });
    Bytes::from(format!(
        "event: content_block_delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn ant_input_json_delta(index: u32, partial_json: &str) -> Bytes {
    let json = serde_json::json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {"type": "input_json_delta", "partial_json": partial_json}
    });
    Bytes::from(format!(
        "event: content_block_delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn ant_content_block_stop(index: u32) -> Bytes {
    let json = serde_json::json!({"type": "content_block_stop", "index": index});
    Bytes::from(format!(
        "event: content_block_stop\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn ant_message_stop() -> Bytes {
    let json = serde_json::json!({"type": "message_stop"});
    Bytes::from(format!(
        "event: message_stop\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn ant_message_delta() -> Bytes {
    let json = serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}});
    Bytes::from(format!(
        "event: message_delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn openai_tool_call_delta(tc_index: u32, arguments: &str) -> Bytes {
    let json = serde_json::json!({
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": tc_index, "function": {"arguments": arguments}}]}, "finish_reason": null}]
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn openai_finish_reason(reason: &str) -> Bytes {
    let json = serde_json::json!({
        "choices": [{"index": 0, "delta": {}, "finish_reason": reason}]
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn openai_done() -> Bytes {
    Bytes::from("data: [DONE]\n\n")
}

fn openai_done_spaceless() -> Bytes {
    Bytes::from("data:[DONE]\n\n")
}

fn responses_text_delta(delta: &str) -> Bytes {
    let json = serde_json::json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": delta});
    Bytes::from(format!(
        "event: response.output_text.delta\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn responses_content_part_done() -> Bytes {
    let json = serde_json::json!({"type": "response.content_part.done", "output_index": 0, "content_index": 0});
    Bytes::from(format!(
        "event: response.content_part.done\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn responses_output_item_done() -> Bytes {
    let json = serde_json::json!({"type": "response.output_item.done", "output_index": 0});
    Bytes::from(format!(
        "event: response.output_item.done\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn responses_completed() -> Bytes {
    let json = serde_json::json!({"type": "response.completed", "response": {"id": "resp_test", "status": "completed"}});
    Bytes::from(format!(
        "event: response.completed\ndata: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn gemini_text_frame(text: &str) -> Bytes {
    let json = serde_json::json!({
        "candidates": [{"content": {"parts": [{"text": text}], "role": "model"}, "index": 0}]
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn gemini_terminal() -> Bytes {
    let json = serde_json::json!({"candidates": [{"finishReason": "STOP", "index": 0}]});
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn gemini_colocated(text: &str) -> Bytes {
    let json = serde_json::json!({
        "candidates": [{
            "content": {"parts": [{"text": text}], "role": "model"},
            "finishReason": "STOP",
            "index": 0
        }]
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

fn gemini_text_with_grounding(text: &str) -> Bytes {
    let json = serde_json::json!({
        "candidates": [{"content": {"parts": [{"text": text}], "role": "model"}, "index": 0}],
        "groundingMetadata": {"webSearchQueries": ["example query"]}
    });
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&json).unwrap()
    ))
}

/// VC-SSE-14: synthetic frames for block N MUST appear before `content_block_stop` for N.
#[tokio::test]
async fn vc_sse_14_anthropic_block_stop_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    assert!(max_fake_len > 0, "fake must have non-zero length");
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();

    // Split fake into 3 parts totaling exactly max_fake_len — no safe-prefix flush fires.
    let a = fake_str.len() / 3;
    let b = 2 * fake_str.len() / 3;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Anthropic,
    );

    tx.send(Ok(ant_input_json_delta(0, &fake_str[..a])))
        .unwrap();
    tx.send(Ok(ant_input_json_delta(0, &fake_str[a..b])))
        .unwrap();
    tx.send(Ok(ant_input_json_delta(0, &fake_str[b..])))
        .unwrap();
    tx.send(Ok(ant_content_block_stop(0))).unwrap();
    tx.send(Ok(ant_message_stop())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-14: fake must not appear in output; fake: {fake_str:?}"
    );
    let sp = pos(&output, secret_str);
    let cp = pos(&output, "content_block_stop");
    assert!(
        sp < cp,
        "VC-SSE-14: secret (pos {sp}) MUST appear before content_block_stop (pos {cp})"
    );
}

/// VC-SSE-15: synthetic frames MUST appear before `message_stop` and before `message_delta`.
#[tokio::test]
async fn vc_sse_15_anthropic_message_stop_ordering() {
    let secret_str = std::str::from_utf8(SECRET).unwrap();

    // Sub-case A: text_delta → message_stop
    {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: entries[0].fake must equal max_fake_len"
        );
        let mid = fake_str.len() / 2;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::Anthropic,
        );

        tx.send(Ok(ant_text_delta(0, &fake_str[..mid]))).unwrap();
        tx.send(Ok(ant_text_delta(0, &fake_str[mid..]))).unwrap();
        tx.send(Ok(ant_message_stop())).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        let sp = pos(&output, secret_str);
        let mp = pos(&output, "message_stop");
        assert!(
            !output.contains(&fake_str),
            "VC-SSE-15 (message_stop): fake must not appear in output"
        );
        assert!(
            sp < mp,
            "VC-SSE-15 (message_stop): secret (pos {sp}) MUST appear before message_stop (pos {mp})"
        );
    }

    // Sub-case B: text_delta → message_delta
    {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: entries[0].fake must equal max_fake_len"
        );
        let mid = fake_str.len() / 2;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::Anthropic,
        );

        tx.send(Ok(ant_text_delta(0, &fake_str[..mid]))).unwrap();
        tx.send(Ok(ant_text_delta(0, &fake_str[mid..]))).unwrap();
        tx.send(Ok(ant_message_delta())).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        let sp = pos(&output, secret_str);
        let mp = pos(&output, "message_delta");
        assert!(
            !output.contains(&fake_str),
            "VC-SSE-15 (message_delta): fake must not appear in output"
        );
        assert!(
            sp < mp,
            "VC-SSE-15 (message_delta): secret (pos {sp}) MUST appear before message_delta (pos {mp})"
        );
    }
}

/// VC-SSE-16: `content_block_stop` for block N MUST NOT flush buffers for block M≠N.
#[tokio::test]
async fn vc_sse_16_anthropic_block_stop_isolation() {
    const MARKER: &str = "VC16_BLOCK1_MARKER";

    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    // MARKER is shorter than max_fake_len so it stays fully held until EOF.
    assert!(
        MARKER.len() < max_fake_len,
        "marker ({}) must be shorter than max_fake_len ({max_fake_len}) for the hold invariant",
        MARKER.len()
    );
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let a = fake_str.len() / 3;
    let b = 2 * fake_str.len() / 3;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Anthropic,
    );

    // Block 0: text_delta frames carrying the fake (total == max_fake_len).
    tx.send(Ok(ant_text_delta(0, &fake_str[..a]))).unwrap();
    tx.send(Ok(ant_text_delta(0, &fake_str[a..b]))).unwrap();
    tx.send(Ok(ant_text_delta(0, &fake_str[b..]))).unwrap();
    // Block 1: single input_json_delta frame with the marker (shorter than max_fake_len).
    tx.send(Ok(ant_input_json_delta(1, MARKER))).unwrap();
    // Stop only block 0; block 1 must NOT be flushed here.
    tx.send(Ok(ant_content_block_stop(0))).unwrap();
    // EOF — block 1 drains here.
    drop(tx);

    let output = collect_all(restore).await;

    let sp = pos(&output, secret_str);
    let cp = pos(&output, "content_block_stop");
    let mp = pos(&output, MARKER);

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-16: fake must not appear in output"
    );
    assert!(
        sp < cp,
        "VC-SSE-16: block 0 secret (pos {sp}) MUST appear before content_block_stop (pos {cp})"
    );
    assert!(
        mp > cp,
        "VC-SSE-16: block 1 marker (pos {mp}) MUST NOT appear before content_block_stop \
         (pos {cp}) — block-stop isolation violated"
    );
    assert!(
        output.contains(MARKER),
        "VC-SSE-16: block 1 marker MUST appear in output (drained at EOF)"
    );
}

/// VC-SSE-17: synthetic frames MUST appear before `finish_reason` and before `[DONE]`.
#[tokio::test]
async fn vc_sse_17_openai_finish_reason_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenAi,
    );

    // Split fake across two tool_call delta frames (total == max_fake_len).
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[..mid])))
        .unwrap();
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[mid..])))
        .unwrap();
    tx.send(Ok(openai_finish_reason("tool_calls"))).unwrap();
    tx.send(Ok(openai_done())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-17: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let fp = pos(&output, "finish_reason");
    let dp = pos(&output, "[DONE]");
    assert!(
        sp < fp,
        "VC-SSE-17: secret (pos {sp}) MUST appear before finish_reason (pos {fp})"
    );
    assert!(
        sp < dp,
        "VC-SSE-17: secret (pos {sp}) MUST appear before [DONE] (pos {dp})"
    );
}

/// VC-SSE-17b: `[DONE]` alone (no preceding `finish_reason`) MUST still flush all buffers.
///
/// This test sends ONLY `data: [DONE]` with no preceding finish_reason frame.
/// Deleting the `[DONE]` flush branch in `process_one_frame` MUST cause this test to FAIL.
#[tokio::test]
async fn vc_sse_17b_openai_done_only_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenAi,
    );

    // Pre-load accumulator with a split fake, then send ONLY [DONE] (no finish_reason).
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[..mid])))
        .unwrap();
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[mid..])))
        .unwrap();
    tx.send(Ok(openai_done())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-17b: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let dp = pos(&output, "[DONE]");
    assert!(
        sp < dp,
        "VC-SSE-17b: secret (pos {sp}) MUST appear before [DONE] (pos {dp})"
    );
}

/// VC-SSE-17c: spaceless `data:[DONE]` (no space) MUST flush all accumulators,
/// identical to `data: [DONE]` (with space).
///
/// Removing the `.or_else(|| l.strip_prefix("data:"))` fallback in
/// `process_one_frame` MUST cause this test to FAIL.
#[tokio::test]
async fn vc_sse_17c_openai_done_spaceless_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenAi,
    );

    // Pre-load accumulator with a split fake, then send spaceless [DONE].
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[..mid])))
        .unwrap();
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[mid..])))
        .unwrap();
    tx.send(Ok(openai_done_spaceless())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-17c: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let dp = pos(&output, "[DONE]");
    assert!(
        sp < dp,
        "VC-SSE-17c: secret (pos {sp}) MUST appear before spaceless [DONE] (pos {dp})"
    );
}

/// VC-SSE-18: synthetic frames MUST appear before each Responses API terminal event.
///
/// Three independent sub-cases each preload a fresh accumulator to `max_fake_len` bytes
/// before a single terminal, so no sub-case's assertions are vacuous due to an earlier flush.
#[tokio::test]
async fn vc_sse_18_responses_api_done_ordering() {
    let secret_str = std::str::from_utf8(SECRET).unwrap();

    // Sub-case A: delta×3 → response.content_part.done
    {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        assert!(
            !result.entries.is_empty(),
            "doppel must detect the synthetic key"
        );
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: fake must equal max_fake_len (content_part.done)"
        );
        let a = fake_str.len() / 3;
        let b = 2 * fake_str.len() / 3;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::OpenAi,
        );

        tx.send(Ok(responses_text_delta(&fake_str[..a]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[a..b]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[b..]))).unwrap();
        tx.send(Ok(responses_content_part_done())).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        assert!(
            !output.contains(&fake_str),
            "VC-SSE-18 (content_part.done): fake must not appear in output"
        );
        let sp = pos(&output, secret_str);
        let cp = pos(&output, "response.content_part.done");
        assert!(
            sp < cp,
            "VC-SSE-18: secret (pos {sp}) MUST appear before response.content_part.done (pos {cp})"
        );
    }

    // Sub-case B: delta×3 → response.output_item.done
    {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        assert!(
            !result.entries.is_empty(),
            "doppel must detect the synthetic key"
        );
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: fake must equal max_fake_len (output_item.done)"
        );
        let a = fake_str.len() / 3;
        let b = 2 * fake_str.len() / 3;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::OpenAi,
        );

        tx.send(Ok(responses_text_delta(&fake_str[..a]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[a..b]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[b..]))).unwrap();
        tx.send(Ok(responses_output_item_done())).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        assert!(
            !output.contains(&fake_str),
            "VC-SSE-18 (output_item.done): fake must not appear in output"
        );
        let sp = pos(&output, secret_str);
        let ip = pos(&output, "response.output_item.done");
        assert!(
            sp < ip,
            "VC-SSE-18: secret (pos {sp}) MUST appear before response.output_item.done (pos {ip})"
        );
    }

    // Sub-case C: delta×3 → response.completed
    {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        assert!(
            !result.entries.is_empty(),
            "doppel must detect the synthetic key"
        );
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: fake must equal max_fake_len (response.completed)"
        );
        let a = fake_str.len() / 3;
        let b = 2 * fake_str.len() / 3;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::OpenAi,
        );

        tx.send(Ok(responses_text_delta(&fake_str[..a]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[a..b]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[b..]))).unwrap();
        tx.send(Ok(responses_completed())).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        assert!(
            !output.contains(&fake_str),
            "VC-SSE-18 (response.completed): fake must not appear in output"
        );
        let sp = pos(&output, secret_str);
        let rp = pos(&output, "response.completed");
        assert!(
            sp < rp,
            "VC-SSE-18: secret (pos {sp}) MUST appear before response.completed (pos {rp})"
        );
    }
}

/// VC-SSE-19: a Gemini frame with co-located content + `finishReason` MUST be routed
/// as path B (content accumulation), not path A (terminal-only handling).
///
/// Assertions on content accumulation pass today. The `finishReason` preservation
/// is a known gap (see §Known Limitations).
#[tokio::test]
async fn vc_sse_19_gemini_colocated_content_is_accumulated() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    let secret_str = std::str::from_utf8(SECRET).unwrap();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Gemini,
    );

    tx.send(Ok(gemini_colocated(&fake_str))).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        output.contains(secret_str),
        "VC-SSE-19: secret MUST appear in output; content was dropped (path-A misrouting)"
    );
    assert!(
        !output.contains(&fake_str),
        "VC-SSE-19: fake must not appear in output"
    );
}

/// VC-SSE-19 (gap): co-located `finishReason` is currently dropped by path-B.
#[tokio::test]
#[ignore = "known gap: co-located finishReason dropped by path-B (see SPEC.md §Known Limitations)"]
async fn vc_sse_19_gemini_colocated_finish_reason_preserved() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let fake_str = String::from_utf8(result.entries[0].fake.clone()).unwrap();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Gemini,
    );

    tx.send(Ok(gemini_colocated(&fake_str))).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        output.contains("STOP"),
        "finishReason must be present in output (currently a known gap)"
    );
}

/// VC-SSE-20: accumulated content MUST appear before a Gemini empty-terminal frame.
#[tokio::test]
async fn vc_sse_20_gemini_empty_terminal_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Gemini,
    );

    // Split fake across two text-part frames (total == max_fake_len).
    tx.send(Ok(gemini_text_frame(&fake_str[..mid]))).unwrap();
    tx.send(Ok(gemini_text_frame(&fake_str[mid..]))).unwrap();
    // Empty terminal: no extractable content — triggers stream flush before forwarding.
    tx.send(Ok(gemini_terminal())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    let sp = pos(&output, secret_str);
    let fp = pos(&output, "finishReason");
    assert!(
        !output.contains(&fake_str),
        "VC-SSE-20: fake must not appear in output"
    );
    assert!(
        sp < fp,
        "VC-SSE-20: secret (pos {sp}) MUST appear before finishReason (pos {fp})"
    );
}

/// VC-SSE-12 extension: Gemini deferred metadata (`groundingMetadata`) MUST appear
/// between the restored text flush and the finishReason terminal frame.
///
/// Exercises the deferred-metadata drain introduced alongside VC-SSE-20 (Terminal
/// Event Ordering for Gemini).
///
/// Deleting the `deferred_passthrough.drain(..)` call in the Gemini terminal branch
/// of `process_one_frame` MUST cause this test to FAIL.
#[tokio::test]
async fn vc_sse_12_gemini_deferred_metadata_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: entries[0].fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::Gemini,
    );

    // First half: plain text frame (accumulates, no deferred).
    tx.send(Ok(gemini_text_frame(&fake_str[..mid]))).unwrap();
    // Second half: text + groundingMetadata (path B — text accumulates, grounding deferred).
    tx.send(Ok(gemini_text_with_grounding(&fake_str[mid..])))
        .unwrap();
    // Content-less terminal: flush text → deferred metadata → terminal.
    tx.send(Ok(gemini_terminal())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-12: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let gp = pos(&output, "groundingMetadata");
    let fp = pos(&output, "finishReason");
    assert!(
        sp < gp,
        "VC-SSE-12: secret (pos {sp}) MUST appear before groundingMetadata (pos {gp})"
    );
    assert!(
        gp < fp,
        "VC-SSE-12: groundingMetadata (pos {gp}) MUST appear before finishReason (pos {fp})"
    );
}

/// VC-SSE-17d: `[DONE]` alone MUST flush all buffers under `Provider::OpenRouter`,
/// identical to the OpenAI case covered by `vc_sse_17b`.
#[tokio::test]
async fn vc_sse_17d_openrouter_done_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(
        fake_str.len(),
        max_fake_len,
        "sizing invariant: fake must equal max_fake_len"
    );
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenRouter,
    );

    tx.send(Ok(openai_tool_call_delta(0, &fake_str[..mid])))
        .unwrap();
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[mid..])))
        .unwrap();
    tx.send(Ok(openai_done())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-17d: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let dp = pos(&output, "[DONE]");
    assert!(
        sp < dp,
        "VC-SSE-17d: secret (pos {sp}) MUST appear before [DONE] (pos {dp})"
    );
}

/// VC-SSE-18b: error Responses API terminals (`response.failed`, `response.cancelled`,
/// `response.incomplete`) MUST also flush all accumulators before being forwarded.
#[tokio::test]
async fn vc_sse_18b_responses_api_error_terminal_ordering() {
    for (event_type, search_str) in [
        ("response.failed", "response.failed"),
        ("response.cancelled", "response.cancelled"),
        ("response.incomplete", "response.incomplete"),
    ] {
        let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
        assert!(
            !result.entries.is_empty(),
            "doppel must detect the synthetic key"
        );
        let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
        let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
        let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
        assert_eq!(
            fake_str.len(),
            max_fake_len,
            "sizing invariant: fake must equal max_fake_len ({event_type})"
        );
        let secret_str = std::str::from_utf8(SECRET).unwrap();
        let a = fake_str.len() / 3;
        let b = 2 * fake_str.len() / 3;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
        let restore = SseRestoreStream::new(
            unbounded_receiver_to_stream(rx),
            result.entries,
            result.session_key,
            Provider::OpenAi,
        );

        tx.send(Ok(responses_text_delta(&fake_str[..a]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[a..b]))).unwrap();
        tx.send(Ok(responses_text_delta(&fake_str[b..]))).unwrap();
        let error_frame = {
            let json = serde_json::json!({"type": event_type});
            bytes::Bytes::from(format!(
                "event: {event_type}\ndata: {}\n\n",
                serde_json::to_string(&json).unwrap()
            ))
        };
        tx.send(Ok(error_frame)).unwrap();
        drop(tx);

        let output = collect_all(restore).await;

        assert!(
            !output.contains(&fake_str),
            "VC-SSE-18b ({event_type}): fake must not appear in output"
        );
        let sp = pos(&output, secret_str);
        let ep = pos(&output, search_str);
        assert!(
            sp < ep,
            "VC-SSE-18b ({event_type}): secret (pos {sp}) MUST appear before {event_type} (pos {ep})"
        );
    }
}

/// VC-SSE-17e: OpenRouter/OpenAI finish_reason frames with `"content":""` (empty string)
/// MUST route to path A (classify_terminal) and flush all buffers before forwarding.
///
/// Real OpenRouter upstreams co-locate `"content":""` with `"finish_reason":"stop"` in the
/// final Chat Completions chunk. If extract_fields treats the empty content as extractable,
/// the frame routes to path B and classify_terminal is never called — the terminal flush
/// does not fire and held content is never restored.
#[tokio::test]
async fn vc_sse_17e_openrouter_empty_content_finish_reason_ordering() {
    let result = doppel::swap(SECRET, &doppel::patterns::all()).unwrap();
    assert!(
        !result.entries.is_empty(),
        "doppel must detect the synthetic key"
    );
    let max_fake_len = result.entries.iter().map(|e| e.fake.len()).max().unwrap();
    let fake_entry = result.entries.iter().max_by_key(|e| e.fake.len()).unwrap();
    let fake_str = String::from_utf8(fake_entry.fake.clone()).unwrap();
    assert_eq!(fake_str.len(), max_fake_len, "sizing invariant");
    let secret_str = std::str::from_utf8(SECRET).unwrap();
    let mid = fake_str.len() / 2;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, io::Error>>();
    let restore = SseRestoreStream::new(
        unbounded_receiver_to_stream(rx),
        result.entries,
        result.session_key,
        Provider::OpenRouter,
    );

    // Feed tool-call deltas carrying the fake key
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[..mid])))
        .unwrap();
    tx.send(Ok(openai_tool_call_delta(0, &fake_str[mid..])))
        .unwrap();

    // Final chunk: co-located empty content + non-null finish_reason (real OpenRouter wire format).
    // The empty content MUST NOT prevent classify_terminal from running.
    let finish_frame = Bytes::from(
        "data: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}]}\n\n",
    );
    tx.send(Ok(finish_frame)).unwrap();
    tx.send(Ok(openai_done())).unwrap();
    drop(tx);

    let output = collect_all(restore).await;

    assert!(
        !output.contains(&fake_str),
        "VC-SSE-17e: fake must not appear in output"
    );
    let sp = pos(&output, secret_str);
    let fp = pos(&output, "\"finish_reason\":");
    assert!(
        sp < fp,
        "VC-SSE-17e: secret (pos {sp}) MUST appear before finish_reason (pos {fp})"
    );
}
