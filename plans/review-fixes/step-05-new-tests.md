# Step 05 — Test improvements (B1 test fix, I4 new integration test)

**Files:**
- `crates/lcp-server/src/ext/sse_unscrub.rs` (B1 test strengthening)
- `tests/integration/scrub.rs` (I4 new integration test)

**Wave:** 1 (depends on wave 0 — code fixes must be in place)

---

## B1 — Strengthen existing unit test `sse_event_lines_preserved_through_text_frame_reconstruction`

### Problem

The existing test at `sse_unscrub.rs:589` only asserts:
```rust
assert!(out_str.contains("event: content_block_delta\n"), ...);
assert!(out_str.contains("event: message_stop\n"));
```

This does not catch the B1 bug because it doesn't verify that `event:` and `data:`
are in the **same frame** (no `\n\n` between them). A frame split by a spurious
`\n\n` would still contain both substrings.

### Change

Replace the two contains assertions (lines 612-617) with assertions that verify
frame integrity — the `event:` line and its `data:` line must be in the same SSE
frame (separated by `\n`, not `\n\n`):

```rust
        // Each event: + data: pair must be in the same SSE frame (no \n\n between them).
        // Split on the frame delimiter and check each frame.
        let frames: Vec<&str> = out_str.split("\n\n").filter(|f| !f.is_empty()).collect();
        let text_frame = frames
            .iter()
            .find(|f| f.contains("content_block_delta"))
            .expect("must have a content_block_delta frame");
        assert!(
            text_frame.contains("event: content_block_delta\n")
                && text_frame.contains("data: "),
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
```

---

## I4 — New integration test: Anthropic SSE with `event:` prefix lines

### Problem

The existing integration test `unscrub_restores_secret_from_anthropic_sse_stream`
(line 711) formats SSE chunks as `"data: {..}\n\n"` without `event:` prefix lines.
The `event: ` detection fix (commit `6e39cc0`) is exercised only by a unit test of
the detector. A regression in the `event: ` branch would not be caught end-to-end.

### New test

Add `unscrub_restores_secret_from_anthropic_sse_stream_with_event_prefix` after the
existing test (after line 790). Same structure as the existing Anthropic test but
with every SSE chunk formatted as `"event: <name>\ndata: {..}\n\n"` to match what
Anthropic's real API sends.

```rust
#[tokio::test]
async fn unscrub_restores_secret_from_anthropic_sse_stream_with_event_prefix() {
    // Regression test: Anthropic's real API prefixes every data line with an
    // event: line. The SSE detector must recognize `event: ` as SSE, and the
    // unscrub pipeline must handle the event: prefix lines correctly.
    use its_classified::scrub as ic_scrub;

    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = ic_scrub(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = vec![
        format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null}}}}\n\n"
        ),
        format!(
            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n"
        ),
    ];
    for part in &parts {
        sse_chunks.push(format!(
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{part}\"}}}}\n\n"
        ));
    }
    sse_chunks.push(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            .to_owned(),
    );
    sse_chunks.push(
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n"
            .to_owned(),
    );
    sse_chunks.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(ScrubExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"claude-haiku-4-5","max_tokens":200,"stream":true,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(ANT)
    );

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp_bytes = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_present(
        &resp_bytes,
        &[ANT],
        "client SSE response: Phase 3 must restore original secret from event-prefixed stream",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client SSE response: scrubbed fake must not be visible to client",
    );
}
```

---

## Acceptance

```bash
cargo nextest run -p lcp-server --lib -- sse_event_lines_preserved
cargo nextest run --test integration -- unscrub_restores_secret_from_anthropic_sse_stream_with_event_prefix
cargo nextest run
cargo clippy --workspace --all-targets -- -D warnings
```

All tests pass, including the two new/improved tests.
