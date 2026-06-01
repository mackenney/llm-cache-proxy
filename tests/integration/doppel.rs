//! Swap/restore integration tests.
//!
//! Exercises `DoppelExt` (backed by `doppel`) end-to-end through the
//! proxy pipeline:
//!   - Phase 2: `doppel::scrub` replaces real secrets with fakes before
//!     the upstream sees the body.
//!   - Phase 3: `doppel::unscrub_stream` restores originals in the
//!     response stream before the response is cached and returned.
//!
//! SPEC ref: lcp-server/SPEC.md §Pipeline and Cache Interaction
//!   "For a swap/restore extension pair, Phase 3 restores the original bytes,
//!    so the cache stores originals while the wire carried only fakes."

use doppel::{register, patterns};
use lcp_server::{ExtensionPipeline, DoppelExt};

use crate::common::{MockUpstream, TestHarness};

// ---------------------------------------------------------------------------
// Synthetic test secrets — NOT real credentials.
// Structures match the built-in patterns of its-classified exactly.
// ---------------------------------------------------------------------------

/// Anthropic API key (sk-ant-api03-…)
const ANT: &[u8] =
    b"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// OpenAI classic key (sk-…)
const OPENAI_CLASSIC: &[u8] = b"sk-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

// OpenAI project key is constructed at test time — see wire_openai_project_key_swapped.

/// AWS AKIA access key ID
const AWS_AKIA: &[u8] = b"AKIAIOSFODNN7EXAMPLE";

/// GitHub classic PAT (ghp_…)
const GITHUB_CLASSIC: &[u8] = b"ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// GitHub fine-grained PAT (github_pat_…_…)
const GITHUB_FG: &[u8] =
    b"github_pat_AAAAAAAAAAAAAAAAAAAAAA_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

/// GCP API key (AIzaSy…)
const GCP: &[u8] = b"AIzaSyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// registered: arbitrary token long enough for registration (≥ 14 variable bytes)
const TIER2_TOKEN: &[u8] = b"my-internal-bearer-token-abcdef1234567890-for-e2e-tests";

/// registered: a UUID-style identifier (enough bytes for registered registration)
const TIER2_UUID_LIKE: &[u8] = b"f47ac10b-58cc-4372-a567-0e02b2c3d479abcdef0123456789";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Assert that `haystack` contains none of the byte sequences in `needles`.
fn assert_absent(haystack: &[u8], needles: &[&[u8]], ctx: &str) {
    for needle in needles {
        assert!(
            !haystack.windows(needle.len()).any(|w| w == *needle),
            "{ctx}: real secret must NOT appear in output",
        );
    }
}

/// Assert that `haystack` contains all of the byte sequences in `needles`.
fn assert_present(haystack: &[u8], needles: &[&[u8]], ctx: &str) {
    for needle in needles {
        assert!(
            haystack.windows(needle.len()).any(|w| w == *needle),
            "{ctx}: expected secret to appear in output",
        );
    }
}

// ---------------------------------------------------------------------------
// Per-morphology wire tests: upstream receives a fake, never the real secret
// ---------------------------------------------------------------------------

macro_rules! wire_doppel_test {
    ($name:ident, $secret:expr, $patterns:expr) => {
        #[tokio::test]
        async fn $name() {
            let secret: &[u8] = $secret;
            let pats: Vec<_> = $patterns;

            let mock = MockUpstream::builder()
                .json(200, r#"{"result":"ok"}"#)
                .build()
                .await;
            let harness = TestHarness::builder()
                .mock(mock)
                .extensions(ExtensionPipeline::new().register(DoppelExt::new(pats)))
                .build()
                .await;
            let client = reqwest::Client::new();

            // Embed the secret in the request body.
            let body = format!(
                r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"{}"}}]}}"#,
                String::from_utf8_lossy(secret)
            );

            let resp = client
                .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .unwrap();

            assert_eq!(resp.status(), 200);
            let _ = resp.bytes().await.unwrap();
            harness.wait_for_writes().await;

            let upstream_requests = harness.mock_requests();
            assert_eq!(upstream_requests.len(), 1, "upstream must receive exactly one request");
            let upstream_body = upstream_requests[0].body.as_ref();

            assert_absent(upstream_body, &[secret], "upstream body");
        }
    };
}

wire_doppel_test!(
    wire_anthropic_key_swapped,
    ANT,
    vec![patterns::anthropic()]
);
wire_doppel_test!(
    wire_openai_classic_key_swapped,
    OPENAI_CLASSIC,
    vec![patterns::openai_classic()]
);

wire_doppel_test!(wire_aws_akia_swapped, AWS_AKIA, vec![patterns::aws_akia()]);
wire_doppel_test!(
    wire_github_classic_pat_swapped,
    GITHUB_CLASSIC,
    vec![patterns::github_classic()]
);
wire_doppel_test!(
    wire_github_fine_grained_pat_swapped,
    GITHUB_FG,
    vec![patterns::github_fine_grained()]
);
wire_doppel_test!(wire_gcp_key_swapped, GCP, vec![patterns::gcp()]);
wire_doppel_test!(
    wire_registered_arbitrary_token_swapped,
    TIER2_TOKEN,
    vec![register(TIER2_TOKEN).expect("registered register failed")]
);
wire_doppel_test!(
    wire_registered_uuid_like_swapped,
    TIER2_UUID_LIKE,
    vec![register(TIER2_UUID_LIKE).expect("registered register failed")]
);

// OpenAI project key: sk-proj- (8) + 58 url-safe-base64 chars + T3BlbkFJ (8) + 58 more = 132 total.
// Constructed here to avoid miscounting in a raw byte string.
#[tokio::test]
async fn wire_openai_project_key_swapped() {
    let mut key: Vec<u8> = b"sk-proj-".to_vec();
    key.extend(std::iter::repeat_n(b'A', 58));
    key.extend_from_slice(b"T3BlbkFJ");
    key.extend(std::iter::repeat_n(b'B', 58));

    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(
            ExtensionPipeline::new().register(DoppelExt::new(vec![patterns::openai_project()])),
        )
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(&key)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let reqs = harness.mock_requests();
    let upstream_body = reqs[0].body.as_ref();
    assert_absent(upstream_body, &[&key], "upstream body (openai project key)");
}

// ---------------------------------------------------------------------------
// Multiple Tier 1 secrets in the same payload — all scrubbed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wire_multiple_secrets_all_swapped() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![
            patterns::anthropic(),
            patterns::aws_akia(),
        ])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":5,"messages":[{{"role":"user","content":"ant={} aws={}"}}]}}"#,
        String::from_utf8_lossy(ANT),
        String::from_utf8_lossy(AWS_AKIA)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let reqs = harness.mock_requests();
    let upstream_body = reqs[0].body.as_ref();
    assert_absent(
        upstream_body,
        &[ANT, AWS_AKIA],
        "upstream body (multi-secret)",
    );
}

// ---------------------------------------------------------------------------
// Tier 1 + registered in same payload
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wire_pattern_and_registered_in_same_payload() {
    let pat2 = register(TIER2_TOKEN).expect("registered register");

    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(
            ExtensionPipeline::new().register(DoppelExt::new(vec![patterns::anthropic(), pat2])),
        )
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":5,"messages":[{{"role":"user","content":"key={} tok={}"}}]}}"#,
        String::from_utf8_lossy(ANT),
        String::from_utf8_lossy(TIER2_TOKEN)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let reqs = harness.mock_requests();
    let upstream_body = reqs[0].body.as_ref();
    assert_absent(
        upstream_body,
        &[ANT, TIER2_TOKEN],
        "upstream body (tier1+registered)",
    );
}

// ---------------------------------------------------------------------------
// Unregistered secret passes through untouched
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wire_unregistered_secret_passes_through() {
    // Only ANT is registered; OPENAI_CLASSIC is not.
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![patterns::anthropic()])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":5,"messages":[{{"role":"user","content":"ant={} oai={}"}}]}}"#,
        String::from_utf8_lossy(ANT),
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let reqs = harness.mock_requests();
    let upstream_body = reqs[0].body.as_ref();
    assert_absent(
        upstream_body,
        &[ANT],
        "upstream body (registered secret must be scrubbed)",
    );
    assert_present(
        upstream_body,
        &[OPENAI_CLASSIC],
        "upstream body (unregistered secret must survive)",
    );
}

// ---------------------------------------------------------------------------
// Phase 3: response containing the fake is restored for the client
// ---------------------------------------------------------------------------

#[tokio::test]
async fn client_receives_restored_secret_in_response() {
    // Use the same Pattern instance for pre-computation and DoppelExt so that
    // the fake is identical in both — each patterns::*() call produces a
    // fresh ephemeral salt.
    use doppel::swap as doppel_swap;
    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    let mock_resp = format!(r#"{{"echo":"{fake_str}"}}"#);
    let mock = MockUpstream::builder().json(200, mock_resp).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
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

    assert_present(&resp_bytes, &[ANT], "client response (restored)");
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client response (fake must be gone)",
    );
}

// ---------------------------------------------------------------------------
// Cache stores restored content (originals, not fakes)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_stores_restored_content_not_fake() {
    use doppel::swap as doppel_swap;
    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    let mock_resp = format!(r#"{{"echo":"{fake_str}"}}"#);
    let mock = MockUpstream::builder().json(200, mock_resp).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
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
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "one entry must be in cache");

    let full = harness
        .cache()
        .inspect(&entries[0].key)
        .unwrap()
        .expect("entry must exist");

    let cached: Vec<u8> = full.chunks.iter().flat_map(|c| c.data.bytes()).collect();
    assert_present(&cached, &[ANT], "cached chunks (restored real value)");
    assert_absent(
        &cached,
        &[&fake_bytes],
        "cached chunks (fake must not be stored)",
    );
}

#[tokio::test]
async fn sse_cache_stores_restored_content_not_fake() {
    use doppel::swap as doppel_swap;
    let pat = patterns::openai_classic();
    let body_bytes = [b"key: ".as_slice(), OPENAI_CLASSIC].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 2 delta events to trigger SSE unscrubbing.
    let mid = fake_str.len() / 2;
    let (p1, p2) = (&fake_str[..mid], &fake_str[mid..]);
    let sse_chunks = vec![
        format!(
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{p1}\"}},\"finish_reason\":null}}]}}\n\n"
        ),
        format!(
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{p2}\"}},\"finish_reason\":null}}]}}\n\n"
        ),
        "data: [DONE]\n\n".to_owned(),
    ];

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":200,"stream":true,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "one entry must be in cache");

    let full = harness
        .cache()
        .inspect(&entries[0].key)
        .unwrap()
        .expect("entry must exist");

    let cached: Vec<u8> = full.chunks.iter().flat_map(|c| c.data.bytes()).collect();
    assert_present(
        &cached,
        &[OPENAI_CLASSIC],
        "cached SSE chunks (restored real value)",
    );
    assert_absent(
        &cached,
        &[&fake_bytes],
        "cached SSE chunks (fake must not be stored)",
    );
}

// ---------------------------------------------------------------------------
// Cache HIT replays restored content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cache_hit_replays_restored_content() {
    use doppel::swap as doppel_swap;
    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_str = String::from_utf8_lossy(&sr.entries[0].fake).into_owned();

    let mock_resp = format!(r#"{{"echo":"{fake_str}"}}"#);
    let mock = MockUpstream::builder().json(200, mock_resp).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(ANT)
    );

    let r1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r1.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = r1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r2.headers().get("x-lcp-cache").unwrap(), "HIT");
    let hit_bytes = r2.bytes().await.unwrap();

    assert_present(
        hit_bytes.as_ref(),
        &[ANT],
        "HIT replay (restored real value)",
    );
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "upstream called only on MISS"
    );
}

// ---------------------------------------------------------------------------
// Different secrets → different cache keys
// (swapping is Phase 2 — cache key is derived from the original body)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn different_secrets_produce_different_cache_keys() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"first"}"#)
        .json(200, r#"{"result":"second"}"#)
        .build()
        .await;

    // Pre-compute a second distinct synthetic Anthropic key.
    const ANT2: &[u8] =
        b"sk-ant-api03-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![patterns::anthropic()])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body1 = format!(
        r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(ANT)
    );
    let body2 = format!(
        r#"{{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(ANT2)
    );

    let r1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body1)
        .send()
        .await
        .unwrap();
    let key1 = r1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(r1.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = r1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body2)
        .send()
        .await
        .unwrap();
    let key2 = r2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(r2.headers().get("x-lcp-cache").unwrap(), "MISS");
    let _ = r2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_ne!(
        key1, key2,
        "different secrets must produce different cache keys \
         (swapping is Phase 2 — cache key derived from original body)"
    );
    assert_eq!(harness.cache().list_entries().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// Payload without any detectable secret passes through completely unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn clean_payload_passes_through_unmodified() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"result":"ok"}"#)
        .build()
        .await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(patterns::all())))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":5,"messages":[{"role":"user","content":"hello world"}]}"#;

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let reqs = harness.mock_requests();
    let upstream_body = reqs[0].body.as_ref();
    assert_eq!(
        upstream_body,
        body.as_bytes(),
        "clean payload must reach upstream byte-for-byte unchanged"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 / SSE: fake split across Anthropic content_block_delta events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restore_returns_secret_from_anthropic_sse_stream() {
    // SPEC ref: crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing
    use doppel::swap as doppel_swap;

    // Use the same Pattern instance for both pre-computation and DoppelExt so the
    // fake is derived from the same salt and is therefore identical in both.
    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 content_block_delta events — simulating real Anthropic
    // streaming where the model emits a few characters at a time. 4 fragments is
    // enough to demonstrate the failure; real traffic uses ~30-60 events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = vec![
        format!(
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null}}}}\n\n"
        ),
        format!(
            "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n"
        ),
    ];
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{part}\"}}}}\n\n"
        ));
    }
    sse_chunks.push("data: {\"type\":\"content_block_stop\",\"index\":0}\n\n".to_owned());
    sse_chunks.push(
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n"
            .to_owned(),
    );
    sse_chunks.push("data: {\"type\":\"message_stop\"}\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
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

    // Phase 3 MUST restore the original secret in the SSE response.
    // The fake must not reach the client.
    assert_present(
        &resp_bytes,
        &[ANT],
        "client SSE response: Phase 3 must restore original secret from content_block_delta stream",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client SSE response: scrubbed fake must not be visible to client",
    );
}

#[tokio::test]
async fn restore_returns_secret_from_anthropic_sse_stream_with_event_prefix() {
    // Regression test: Anthropic's real API prefixes every data line with an
    // event: line. The SSE detector must recognize `event: ` as SSE, and the
    // unscrub pipeline must handle the event: prefix lines correctly.
    use doppel::swap as doppel_swap;

    let pat = patterns::anthropic();
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
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
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
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

#[tokio::test]
async fn restore_returns_secret_from_openai_sse_stream() {
    use doppel::swap as doppel_swap;

    let pat = patterns::openai_classic();
    let body_bytes = [b"key: ".as_slice(), OPENAI_CLASSIC].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 delta events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = Vec::new();
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{part}\"}},\"finish_reason\":null}}]}}\n\n"
        ));
    }
    sse_chunks.push("data: {\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_owned());
    sse_chunks.push("data: [DONE]\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":200,"stream":true,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
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
        &[OPENAI_CLASSIC],
        "client OpenAI SSE response: Phase 3 must restore original secret",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client OpenAI SSE response: scrubbed fake must not be visible",
    );
}

#[tokio::test]
async fn restore_returns_secret_from_gemini_sse_stream() {
    use doppel::swap as doppel_swap;

    let pat = patterns::gcp();
    let body_bytes = [b"key: ".as_slice(), GCP].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 delta events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = Vec::new();
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"{part}\"}}],\"role\":\"model\"}},\"finishReason\":\"STOP\",\"index\":0}}]}}\n\n"
        ));
    }

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"contents":[{{"parts":[{{"text":"key={}"}}]}}]}}"#,
        String::from_utf8_lossy(GCP)
    );

    let resp = client
        .post(format!(
            "{}/gemini/v1/models/gemini-2.5-flash:streamGenerateContent",
            harness.proxy_url()
        ))
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
        &[GCP],
        "client Gemini SSE response: Phase 3 must restore original secret",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client Gemini SSE response: scrubbed fake must not be visible",
    );
}

#[tokio::test]
async fn restore_returns_secret_from_openrouter_sse_stream() {
    // OpenRouter uses the same wire format as OpenAI (choices[0].delta.content).
    // SPEC ref: crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing
    use doppel::swap as doppel_swap;

    let pat = patterns::openai_classic();
    let body_bytes = [b"key: ".as_slice(), OPENAI_CLASSIC].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 delta events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = Vec::new();
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{part}\"}},\"finish_reason\":null}}]}}\n\n"
        ));
    }
    sse_chunks.push("data: {\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_owned());
    sse_chunks.push("data: [DONE]\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":200,"stream":true,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let resp = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
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
        &[OPENAI_CLASSIC],
        "client OpenRouter SSE response: Phase 3 must restore original secret",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client OpenRouter SSE response: scrubbed fake must not be visible",
    );
}
