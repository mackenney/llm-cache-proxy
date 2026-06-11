# Step 01: Cassette Infrastructure

## Context

### Why

The Jun 2026 E2E campaign exposed three real bugs that unit tests missed:
1. `flush_safe_prefix` split fakes at JSON object boundaries (`{"key": "sk`|`-ant..."}`)
2. Synthetic Responses API frames used wrong event type (`output_text` instead of `response.output_text.delta`)
3. OpenRouter/OpenAI co-located `"content":""` with `finish_reason` blocked `classify_terminal`

All three were caused by real provider wire formats diverging from synthetic test fixtures.
Cassettes close this gap: record real responses once, replay them deterministically.

### This Step

Implement the cassette infrastructure. No real API calls here — this is pure Rust code
that extends the existing test harness. Step 02 does the recording.

## Files to Read Before Starting

- `tests/common/mock_upstream.rs` — `MockResponse` enum, `MockUpstreamBuilder`, `handle_request`
- `tests/common/harness.rs` — `TestHarness`, `TestHarnessBuilder`
- `tests/common/mod.rs` — public re-exports
- `tests/e2e/sse_fields.rs` — how existing E2E tests use the harness
- `tests/Cargo.toml` — dependencies (add `toml` if absent)
- `AGENTS.md` — project conventions (no section separators, comments explain WHY)

## Implementation

### 1. Cassette file format

Create `tests/fixtures/` directory. Each cassette is one TOML file:

```toml
# Cassette metadata
schema = 1
provider = "anthropic"          # "anthropic" | "openai" | "openrouter" | "gemini"
scenario = "tool_use_input_json_delta"
model = "claude-haiku-4-5"
recorded_at = "2026-06-11"      # date only; no time (stable for version control)
description = """
Anthropic claude-haiku-4-5 streaming tool call. The model echoes the fake key
in input_json_delta chunks. Verifies flush_safe_prefix handles JSON-wrapped fakes.
"""

# Synthetic secret used during recording (NOT a real API key — same constants as
# tests/e2e/sse_fields.rs). The cassette contains the FAKE key (doppel's substitution),
# not the original. On replay, the test embeds the original; the proxy swaps it for
# the fake; the cassette player returns these frames; the proxy restores.
secret_kind = "anthropic"       # drives which doppel::patterns::* to use

[request]
method = "POST"
path = "/v1/messages"           # path forwarded to upstream (without provider prefix)
# Headers are sanitized: Authorization/x-api-key omitted; content-type retained.
# The body is stored WITH the fake already substituted (as the upstream actually receives it).
content_type = "application/json"
body = """
{"model":"claude-haiku-4-5","max_tokens":500,"stream":true,
 "tools":[{"name":"store_key","description":"Store an API key",
           "input_schema":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}],
 "messages":[{"role":"user","content":"Call store_key with this key: sk-ant-api03-FAKEKEY..."}]}
"""

[[response]]
# Each [[response]] table is one HTTP response (for future multi-round scenarios;
# single-request cassettes have exactly one [[response]]).
status = 200

[response.headers]
content-type = "text/event-stream"
# Omit provider-specific headers that change per-request (request-id, etc.)

# body_chunks: the SSE frames exactly as the upstream sent them.
# Each string is one "chunk" — what the upstream wrote to the TCP stream in one write.
# For SSE, each chunk is typically one complete SSE event (ending with \n\n).
# Preserve EXACT whitespace and field ordering; this is the source of truth for
# wire-format coverage.
body_chunks = [
  "event: message_start\ndata: {\"type\":\"message_start\",...}\n\n",
  "event: content_block_start\ndata: {...}\n\n",
  "event: ping\ndata: {\"type\":\"ping\"}\n\n",
  "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"key\\\": \\\"\"}}\n\n",
  "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"sk-ant-api03-FAKE...\"}}\n\n",
  "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"\"}}\n\n",
  "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
  "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
  "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
]
```

**Design decisions:**
- Schema version field allows format evolution without breaking existing cassettes.
- `secret_kind` tells the test which `doppel::patterns::*` function to call to get the
  pattern set; the test then calls `doppel_swap` to compute the fake used in the cassette.
- `body_chunks` stores individual write-level chunks, not the full concatenated body.
  This preserves chunking fidelity (real upstreams don't always send one frame per write).
- No timing metadata — replay is deterministic and instant.
- Real API keys are NEVER stored. The cassette body contains the fake key.
  The cassette is safe to commit.
- Provider-identifying headers (request-id, x-request-id, cf-ray) are stripped.
  content-type is retained. Authorization/x-api-key are never stored.

### 2. Cassette loader

Add `tests/common/cassette.rs`:

```rust
//! Cassette loader: reads fixture TOML files into MockResponse sequences.
//!
//! Cassettes store real provider SSE responses captured with synthetic test keys.
//! The fake key embedded in body_chunks is the doppel substitution of the synthetic
//! secret; the test infrastructure restores it during replay via the proxy.

use std::path::Path;
use bytes::Bytes;

/// A loaded cassette ready to feed into MockUpstreamBuilder.
pub struct Cassette {
    /// Provider that produced this recording.
    pub provider: String,
    /// Human-readable scenario tag.
    pub scenario: String,
    /// Which secret kind was used (drives doppel pattern selection).
    pub secret_kind: String,
    /// HTTP status code to return.
    pub status: u16,
    /// Response headers to return (content-type etc.).
    pub headers: Vec<(String, String)>,
    /// SSE/JSON chunks in order. Each Bytes is one upstream write.
    pub body_chunks: Vec<Bytes>,
}

impl Cassette {
    /// Load a cassette from a TOML fixture file.
    /// Path is relative to the workspace root (e.g. "tests/fixtures/anthropic/tool_use.toml").
    pub fn load(path: impl AsRef<Path>) -> Self { ... }

    /// All chunks concatenated — for non-SSE (JSON) cassettes.
    pub fn full_body(&self) -> Bytes { ... }
}
```

### 3. MockResponse::Cassette variant

In `tests/common/mock_upstream.rs`, add a new variant to `MockResponse`:

```rust
pub enum MockResponse {
    Json { status: u16, body: String },
    Sse { status: u16, chunks: Vec<String> },
    Error { status: u16, body: String },
    Hang,
    /// Stream a pre-recorded cassette. Each chunk is one upstream write.
    /// For SSE cassettes, chunks are complete SSE frames. The `headers` field
    /// carries the content-type and any other response headers to include.
    Recorded {
        status: u16,
        headers: Vec<(String, String)>,
        chunks: Vec<Bytes>,
    },
}
```

Update `handle_request` to handle `MockResponse::Recorded`:

```rust
MockResponse::Recorded { status, headers, chunks } => {
    let stream = futures_util::stream::iter(
        chunks.into_iter().map(Ok::<_, std::convert::Infallible>),
    );
    let mut builder = Response::builder().status(status);
    for (k, v) in &headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    builder.body(Body::from_stream(stream)).unwrap()
}
```

Add to `MockUpstreamBuilder`:

```rust
/// Queue a cassette as the next response.
pub fn cassette(self, c: &Cassette) -> Self {
    self.response(MockResponse::Recorded {
        status: c.status,
        headers: c.headers.clone(),
        chunks: c.body_chunks.clone(),
    })
}
```

### 4. Fixture directory layout

```
tests/fixtures/
  README.md               — how to record new cassettes
  anthropic/
    tool_use_input_json.toml
    thinking_and_text.toml
    multi_block_stop.toml
    empty_response.toml
    error_rate_limit.toml
    error_server_error.toml
  openai/
    chat_tool_calls.toml
    chat_tool_calls_colocated_finish.toml   # finish_reason + content: ""
    chat_finish_reason_stop.toml
    chat_o4mini_tool_calls.toml
    responses_api_text.toml
    responses_api_tool_call.toml
    responses_api_error_incomplete.toml
    error_rate_limit.toml
  openrouter/
    claude_haiku_tool_use.toml              # anthropic model via openrouter
    claude_haiku_finish_colocated.toml      # content:"" + finish_reason (the bug)
    deepseek_chat.toml
    deepseek_r1_reasoning.toml              # reasoning field (not reasoning_content)
    o4mini_tool_calls.toml
    openrouter_processing_prefix.toml       # ": OPENROUTER PROCESSING" comment lines
    error_no_auth.toml
  gemini/
    tool_call.toml
    multi_part_thinking.toml
    colocated_finish_reason.toml            # finishReason co-located with content
    code_execution.toml
    empty_candidate.toml
    error_quota.toml
  README.md
```

`tests/fixtures/README.md` must document:
- How to run a recording session (step-02 binary)
- How to add a new cassette manually
- The `secret_kind` values and their corresponding `doppel::patterns::*` functions
- Naming convention: `{model-slug}_{scenario}.toml`

### 5. Recording binary (`lcp-record`)

Add `src/bin/record.rs` in the `tests` crate (behind `--features record` so it doesn't
ship in normal test builds):

```rust
//! lcp-record: capture live provider responses into cassette fixtures.
//!
//! Usage:
//!   cargo test --test cassette_recorder --features record -- record \
//!     --provider anthropic \
//!     --scenario tool_use_input_json \
//!     --model claude-haiku-4-5 \
//!     --out tests/fixtures/anthropic/tool_use_input_json.toml
//!
//! Environment: ANTHROPIC_API_KEY (or OPENAI_API_KEY etc.) must be set.
//! The recorder injects the synthetic fake key into the request body, forwards
//! to the real upstream, and writes the exact response chunks to the cassette.
```

The recorder:
1. Reads the scenario config (which synthetic key, which request body template)
2. Runs `doppel_swap` on the body to get the fake key
3. Substitutes the fake key into the request body
4. Sends the request to the real upstream (bypassing the proxy — direct HTTP)
5. Captures all response chunks as they arrive
6. Writes the cassette TOML

**Why bypass the proxy for recording?** The cassette must contain what the upstream
actually returns, not what the proxy transforms. The proxy's restoration runs on replay.

### 6. Scenario config registry

Define a `scenarios.toml` in `tests/fixtures/` that lists all recording targets:

```toml
[[scenario]]
id = "anthropic_tool_use_input_json"
provider = "anthropic"
secret_kind = "anthropic"
model = "claude-haiku-4-5"
upstream = "https://api.anthropic.com"
path = "/v1/messages"
method = "POST"
extra_headers = [["anthropic-version", "2023-06-01"]]
body_template = """
{
  "model": "claude-haiku-4-5",
  "max_tokens": 500,
  "stream": true,
  "tools": [{"name":"store_key","description":"Store key","input_schema":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}],
  "messages": [{"role":"user","content":"Call store_key with this key: {{SECRET_PLACEHOLDER}}"}]
}
"""
# {{SECRET_PLACEHOLDER}} is replaced with the fake key before sending to upstream.
# The cassette body_chunks will contain the fake key as the upstream echoes it back.
out = "tests/fixtures/anthropic/tool_use_input_json.toml"
description = "Anthropic tool use — fake key in input_json_delta chunks"

[[scenario]]
id = "openrouter_claude_colocated_finish"
provider = "openrouter"
secret_kind = "openai_classic"
model = "anthropic/claude-haiku-4-5"
upstream = "https://openrouter.ai/api"
path = "/v1/chat/completions"
method = "POST"
extra_headers = []
body_template = """
{
  "model": "anthropic/claude-haiku-4-5",
  "max_tokens": 50,
  "stream": true,
  "tools": [{"type":"function","function":{"name":"store_key","description":"Store","parameters":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}}],
  "messages": [{"role":"user","content":"Call store_key with key: {{SECRET_PLACEHOLDER}}"}]
}
"""
out = "tests/fixtures/openrouter/claude_haiku_finish_colocated.toml"
description = "OpenRouter Claude — co-located content:'' + finish_reason bug scenario"

# ... (30+ scenarios listed in step-02)
```

## Acceptance Criteria

- [ ] `tests/fixtures/` directory created with `README.md`
- [ ] `tests/common/cassette.rs` exists and `Cassette::load` parses the TOML format correctly
- [ ] `MockResponse::Recorded` variant added to `MockUpstream`
- [ ] `MockUpstreamBuilder::cassette()` queues a `Recorded` response
- [ ] `cargo nextest run` exits 0 (all existing tests still pass)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] A dummy cassette in `tests/fixtures/anthropic/dummy.toml` can be loaded and replayed
  through MockUpstream in a new test `cassette_infrastructure::loads_and_replays_dummy`

## Reviewer Instructions

- Confirm `Cassette::load` returns an error (not panic) on missing file or malformed TOML.
- Confirm `MockResponse::Recorded` streams chunks in order, not all-at-once.
- Confirm `MockUpstreamBuilder::cassette()` chains correctly with other builder methods.
- Confirm no real API keys or sensitive data appear in `tests/fixtures/` at this step
  (the dummy cassette uses the ANT/OPENAI_CLASSIC constants from sse_fields.rs).
- Confirm `scenarios.toml` is present and has at least the 30 scenario entries from
  step-02 listed (even if the cassette files themselves don't exist yet).
