//! Extension pipeline spec invariant tests.
//!
//! Covers INV-EXT-1 through INV-EXT-10 from the Extension Pipeline section
//! of `crates/lcp-server/SPEC.md`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;

use lcp_server::{Extension, ExtensionPipeline, ProxyCtx, ResponseStream, SensitiveStateBuilder};

use crate::common::{MockUpstream, TestHarness};

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#
}

async fn send_proxy_request(client: &reqwest::Client, proxy_url: &str) -> reqwest::Response {
    client
        .post(format!("{proxy_url}/anthropic/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap()
}

// INV-EXT-1: Empty pipeline is transparent —————————————————————————————————

#[tokio::test]
async fn inv_ext_1_empty_pipeline_is_transparent() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp.status(), 200, "INV-EXT-1: status must be 200");
    let cache_header = resp
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache header");
    assert_eq!(cache_header, "MISS", "INV-EXT-1: first request is a MISS");

    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "INV-EXT-1: response must be cached");
}

// INV-EXT-2: Phase 1 transforms the cache key input —————————————————————————

struct NormalizeExt;

impl Extension for NormalizeExt {
    fn name(&self) -> &'static str {
        "normalize"
    }

    fn on_request_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<Bytes, anyhow::Error>> {
        // Replace uppercase 'X' (0x58) with 'Y' (0x59) — normalises two
        // logically-equivalent request bodies to the same byte sequence,
        // giving them the same cache key.
        Box::pin(async move {
            let normalized: Vec<u8> = body
                .iter()
                .map(|&b| if b == b'X' { b'Y' } else { b })
                .collect();
            Ok(Bytes::from(normalized))
        })
    }
}

#[tokio::test]
async fn inv_ext_2_phase1_transforms_cache_key_input() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let pipeline = ExtensionPipeline::new().register(NormalizeExt);
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    // Body 1 contains "X" in x_tag; after Phase 1 it becomes "Y".
    let body_with_x =
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[],"x_tag":"X"}"#;
    // Body 2 already has "Y"; after Phase 1 it stays "Y".
    let body_with_y =
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[],"x_tag":"Y"}"#;

    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body_with_x)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp1.headers().get("x-lcp-cache").unwrap(),
        "MISS",
        "INV-EXT-2: first request must be MISS"
    );
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body_with_y)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp2.headers().get("x-lcp-cache").unwrap(),
        "HIT",
        "INV-EXT-2: second request (normalized same body) must be HIT"
    );
}

// INV-EXT-3: Phase 2 fires on miss, not on hit ——————————————————————————————

struct CountPhase2 {
    calls: Arc<AtomicUsize>,
}

impl Extension for CountPhase2 {
    fn name(&self) -> &'static str {
        "count-phase2"
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((body, SensitiveStateBuilder::new()))
        })
    }
}

#[tokio::test]
async fn inv_ext_3_phase2_fires_on_miss_not_hit() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let pipeline = ExtensionPipeline::new().register(CountPhase2 {
        calls: calls.clone(),
    });
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    // First request: MISS — Phase 2 must fire.
    let resp1 = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp1.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-3: Phase 2 must fire exactly once on MISS"
    );

    // Second request: HIT — Phase 2 must NOT fire.
    let resp2 = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp2.headers().get("x-lcp-cache").unwrap(), "HIT");
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-3: Phase 2 must NOT fire on HIT — counter must still be 1"
    );
}

// INV-EXT-4: Phase 3 receives SensitiveState produced by Phase 2 ————————————

struct PingPongExt {
    verified: Arc<AtomicBool>,
}

impl Extension for PingPongExt {
    fn name(&self) -> &'static str {
        "ping-pong"
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move {
            let mut b = SensitiveStateBuilder::new();
            b.set("ping", "pong");
            Ok((body, b))
        })
    }

    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        state: lcp_server::SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        // on_response_stream is called synchronously — state is available right here.
        if state.get("ping") == Some("pong") {
            self.verified.store(true, Ordering::SeqCst);
        }
        stream
    }
}

#[tokio::test]
async fn inv_ext_4_phase3_receives_sensitive_state_from_phase2() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let verified = Arc::new(AtomicBool::new(false));
    let pipeline = ExtensionPipeline::new().register(PingPongExt {
        verified: verified.clone(),
    });
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert!(
        verified.load(Ordering::SeqCst),
        "INV-EXT-4: Phase 3 must receive the SensitiveState produced by Phase 2"
    );
}

// INV-EXT-5: SensitiveState is per-extension ————————————————————————————————

struct IsolationExtA {
    saw_b_key: Arc<AtomicBool>,
}

impl Extension for IsolationExtA {
    fn name(&self) -> &'static str {
        "isolation-a"
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move {
            let mut b = SensitiveStateBuilder::new();
            b.set("key_a", "val_a");
            Ok((body, b))
        })
    }

    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        state: lcp_server::SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        if state.get("key_b").is_some() {
            self.saw_b_key.store(true, Ordering::SeqCst);
        }
        stream
    }
}

struct IsolationExtB {
    saw_a_key: Arc<AtomicBool>,
}

impl Extension for IsolationExtB {
    fn name(&self) -> &'static str {
        "isolation-b"
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move {
            let mut b = SensitiveStateBuilder::new();
            b.set("key_b", "val_b");
            Ok((body, b))
        })
    }

    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        state: lcp_server::SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        if state.get("key_a").is_some() {
            self.saw_a_key.store(true, Ordering::SeqCst);
        }
        stream
    }
}

#[tokio::test]
async fn inv_ext_5_sensitive_state_is_per_extension() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let a_saw_b = Arc::new(AtomicBool::new(false));
    let b_saw_a = Arc::new(AtomicBool::new(false));

    let pipeline = ExtensionPipeline::new()
        .register(IsolationExtA {
            saw_b_key: a_saw_b.clone(),
        })
        .register(IsolationExtB {
            saw_a_key: b_saw_a.clone(),
        });

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert!(
        !a_saw_b.load(Ordering::SeqCst),
        "INV-EXT-5: Extension A must NOT see Extension B's sensitive state"
    );
    assert!(
        !b_saw_a.load(Ordering::SeqCst),
        "INV-EXT-5: Extension B must NOT see Extension A's sensitive state"
    );
}

// INV-EXT-6: Phase 1 fires on bypass requests ———————————————————————————————

struct PhaseCountExt {
    p1_calls: Arc<AtomicUsize>,
    p2_calls: Arc<AtomicUsize>,
}

impl Extension for PhaseCountExt {
    fn name(&self) -> &'static str {
        "phase-count"
    }

    fn on_request_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<Bytes, anyhow::Error>> {
        let p1 = self.p1_calls.clone();
        Box::pin(async move {
            p1.fetch_add(1, Ordering::SeqCst);
            Ok(body)
        })
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        let p2 = self.p2_calls.clone();
        Box::pin(async move {
            p2.fetch_add(1, Ordering::SeqCst);
            Ok((body, SensitiveStateBuilder::new()))
        })
    }
}

#[tokio::test]
async fn inv_ext_6_phase1_fires_on_bypass() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let p1_calls = Arc::new(AtomicUsize::new(0));
    let p2_calls = Arc::new(AtomicUsize::new(0));
    let pipeline = ExtensionPipeline::new().register(PhaseCountExt {
        p1_calls: p1_calls.clone(),
        p2_calls: p2_calls.clone(),
    });

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("x-lcp-bypass", "1")
        .body(request_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers().get("x-lcp-cache").unwrap(), "BYPASS");
    let _ = resp.bytes().await.unwrap();

    assert_eq!(
        p1_calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-6: Phase 1 MUST fire on bypass requests"
    );
    assert_eq!(
        p2_calls.load(Ordering::SeqCst),
        0,
        "INV-EXT-6: Phase 2 MUST NOT fire on bypass requests"
    );
}

// INV-EXT-7: Phase 2 error fails closed, upstream not reached ————————————————

struct FailPhase2Ext;

impl Extension for FailPhase2Ext {
    fn name(&self) -> &'static str {
        "fail-phase2"
    }

    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        _body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move { Err(anyhow::anyhow!("INV-EXT-7: intentional phase 2 failure")) })
    }
}

#[tokio::test]
async fn inv_ext_7_phase2_error_fails_closed() {
    // No queued responses — if the upstream is contacted, mock returns 500.
    // The real test is that mock_requests == 0.
    let mock = MockUpstream::builder().build().await;

    let pipeline = ExtensionPipeline::new().register(FailPhase2Ext);
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    let status = resp.status().as_u16();
    assert!(
        status >= 500,
        "INV-EXT-7: Phase 2 error must produce a 5xx response; got {status}"
    );

    assert_eq!(
        harness.mock_requests().len(),
        0,
        "INV-EXT-7: upstream must NOT be contacted when Phase 2 errors"
    );
}

// INV-EXT-8: Phase 3 wraps the stream that is cached ————————————————————————

struct ByteSubstExt;

impl Extension for ByteSubstExt {
    fn name(&self) -> &'static str {
        "byte-subst"
    }

    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        _state: lcp_server::SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        Box::pin(stream.map(|chunk| {
            chunk.map(|bytes| {
                let replaced: Vec<u8> = bytes
                    .iter()
                    .map(|&b| if b == b'X' { b'Y' } else { b })
                    .collect();
                Bytes::from(replaced)
            })
        }))
    }
}

#[tokio::test]
async fn inv_ext_8_phase3_stream_is_cached() {
    // Upstream returns a body containing 'X'; Phase 3 replaces 'X' with 'Y'.
    // The cached exchange must contain 'Y', not 'X'.
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"XXXXX"}"#)
        .build()
        .await;

    let pipeline = ExtensionPipeline::new().register(ByteSubstExt);
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "INV-EXT-8: one entry must be cached");

    let key = &entries[0].key;
    let full = harness
        .cache()
        .inspect(key)
        .unwrap()
        .expect("INV-EXT-8: entry must exist");

    let all_chunks: String = full.chunks.iter().map(|c| c.data.as_str()).collect();
    assert!(
        all_chunks.contains('Y'),
        "INV-EXT-8: cached chunks must contain 'Y' (Phase 3 transform applied)"
    );
    assert!(
        !all_chunks.contains('X'),
        "INV-EXT-8: cached chunks must NOT contain 'X' (raw upstream bytes must not be stored)"
    );
}

// INV-EXT-9: Phase 1 fires on cache-hit requests ———————————————————————————————

#[tokio::test]
async fn inv_ext_9_phase1_fires_on_cache_hit() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let p1_calls = Arc::new(AtomicUsize::new(0));
    let p2_calls = Arc::new(AtomicUsize::new(0));
    let pipeline = ExtensionPipeline::new().register(PhaseCountExt {
        p1_calls: p1_calls.clone(),
        p2_calls: p2_calls.clone(),
    });

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;

    let client = reqwest::Client::new();

    // First request: MISS — both phases fire.
    let resp1 = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp1.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;
    assert_eq!(
        p1_calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-9: Phase 1 must fire on cache MISS"
    );
    assert_eq!(
        p2_calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-9: Phase 2 must fire on cache MISS"
    );

    // Second request (identical body): HIT — Phase 1 fires, Phase 2 does NOT.
    let resp2 = send_proxy_request(&client, &harness.proxy_url()).await;
    assert_eq!(resp2.headers().get("x-lcp-cache").unwrap(), "HIT");
    let _ = resp2.bytes().await.unwrap();
    // Phase 1 fires synchronously before the cache lookup, so p1_calls is already
    // incremented by the time the response returns. wait_for_writes is still called
    // for consistency with the MISS block above.
    harness.wait_for_writes().await;
    assert_eq!(
        p1_calls.load(Ordering::SeqCst),
        2,
        "INV-EXT-9: Phase 1 MUST fire on cache-hit requests"
    );
    assert_eq!(
        p2_calls.load(Ordering::SeqCst),
        1,
        "INV-EXT-9: Phase 2 MUST NOT fire on cache-hit requests"
    );
}

// INV-EXT-10: Extensions run in registration order ——————————————————————————

struct NameExt {
    name: &'static str,
    order: Arc<Mutex<Vec<String>>>,
}

impl Extension for NameExt {
    fn name(&self) -> &'static str {
        self.name
    }

    fn on_request_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<Bytes, anyhow::Error>> {
        let order = self.order.clone();
        let name = self.name;
        Box::pin(async move {
            order.lock().unwrap().push(name.to_string());
            Ok(body)
        })
    }
}

#[tokio::test]
async fn inv_ext_10_extensions_run_in_registration_order() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;

    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let pipeline = ExtensionPipeline::new()
        .register(NameExt {
            name: "first",
            order: order.clone(),
        })
        .register(NameExt {
            name: "second",
            order: order.clone(),
        });

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(pipeline)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = send_proxy_request(&client, &harness.proxy_url()).await;
    let _ = resp.bytes().await.unwrap();

    let observed = order.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec!["first".to_string(), "second".to_string()],
        "INV-EXT-10: extensions must run in registration order"
    );
}
