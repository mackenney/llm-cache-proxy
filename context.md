# Code Context: GEMINI-1 Known Gap

## Files Retrieved

1. `crates/lcp-server/src/proxy.rs` (lines 57–278) — Entry point; model extraction; cache write flow
2. `crates/lcp-server/src/router.rs` (lines 1–20) — Route handler registration
3. `crates/lcp-server/src/server.rs` (lines 13–46) — ServerConfig struct with upstream fields
4. `crates/lcp-core/src/hash.rs` (lines 1–131) — Cache key computation function
5. `crates/lcp-core/src/cache.rs` (lines 95–125, 165–193) — Cache put() signature; stats() with by_model logic
6. `crates/lcp-core/src/provider.rs` (lines 1–50) — Provider enum and upstream methods
7. `crates/lcp-core/src/types.rs` (lines 1–57) — CacheEntry and FullEntry type definitions
8. `SPEC.md` (lines 31–60) — Providers, upstream URLs, cache key specification
9. `crates/lcp-core/SPEC.md` (lines 47–60) — Cache key normalization rules
10. `crates/lcp-server/SPEC.md` (lines 56–94) — Proxy behavior and model extraction requirement
11. `artifacts/e2e/TEST_RUNS.md` (lines 207–237) — Gemini test execution showing exact URL path
12. `artifacts/e2e/FINDINGS.md` (lines 7–19) — Root cause analysis of GEMINI-1

---

## Key Code Findings

### 1. Model Extraction Function
**File:** `crates/lcp-server/src/proxy.rs`, **lines 274–278**

```rust
fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned))
}
```

**Issue:** Extracts `model` field **only from the JSON request body**. For Gemini, the model is **not** in the body—it's in the URL path.

### 2. Model Storage in Cache
**File:** `crates/lcp-server/src/proxy.rs`, **lines 141, 209–213**

Line 141 extracts model:
```rust
let model = extract_model(&body);
```

Lines 209–213 store it:
```rust
match cache.put(
    &key_clone,
    &provider_prefix,
    model_clone.as_deref(),
    &exchange,
)
```

**File:** `crates/lcp-core/src/cache.rs`, **lines 96–102** — Cache.put() signature:
```rust
pub fn put(
    &self,
    key: &str,
    provider: &str,
    model: Option<&str>,
    exchange: &Exchange,
) -> Result<()>
```

### 3. Exact Gemini URL Path Pattern
**Source:** `artifacts/e2e/TEST_RUNS.md`, **line 215** — Real Gemini test execution:

```
POST http://127.0.0.1:9099/gemini/v1beta/models/gemini-2.5-flash:generateContent?key=<GOOGLE_KEY>
Body: {"contents":[{"parts":[{"text":"Reply with exactly one word: hello"}]}]}
```

**Breaking it down:**
- **Proxy path prefix:** `/gemini/`
- **Remainder passed to upstream:** `v1beta/models/gemini-2.5-flash:generateContent`
- **Model name:** `gemini-2.5-flash` (URL-encoded in path as `/models/{model}:generateContent`)
- **Query params:** `key=<GOOGLE_KEY>` (API key)
- **Request body:** No `model` field; uses `contents` and `parts` structure instead.

### 4. Provider Prefix Stripping & URL Construction
**File:** `crates/lcp-server/src/proxy.rs`, **lines 57–99**

```rust
pub async fn handle(
    State(state): State<AppState>,
    Path((provider_str, path)): Path<(String, String)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    ...
) -> Response {
    // Line 64–70: Parse provider
    let Some(provider) = Provider::from_prefix(&provider_str) else {
        return (StatusCode::NOT_FOUND, format!("unknown provider: {provider_str}")).into_response();
    };

    // Line 78: Reconstruct full path (includes provider prefix)
    let full_path = format!("/{provider_str}/{path}");
    
    // Line 79: Cache key includes full path with provider prefix
    let key = cache_key("POST", &full_path, &body);

    // Line 98: Get upstream base URL
    let upstream = state.config.upstream_for(provider);
    
    // Line 99: Reconstruct upstream URL (provider prefix is STRIPPED)
    let mut url = format!("{}/{}", upstream.trim_end_matches('/'), path);
}
```

**Key observation:**
- **Full path stored in cache key:** `full_path` = `/{provider_str}/{path}` = `/gemini/v1beta/models/gemini-2.5-flash:generateContent`
- **Upstream URL constructed:** `upstream + "/" + path` (provider prefix `provider_str` is **NOT** included)
- For Gemini: upstream = `https://generativelanguage.googleapis.com`, path = `v1beta/models/gemini-2.5-flash:generateContent`
- **Final upstream URL:** `https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent`

### 5. Cache Key Computation
**File:** `crates/lcp-core/src/hash.rs`, **lines 10–19**

```rust
pub fn cache_key(method: &str, path: &str, body: &[u8]) -> String {
    let normalized = normalize_body(body);
    let mut hasher = blake3::Hasher::new();
    hasher.update(method.as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(normalized.as_bytes());
    hasher.finalize().to_hex().to_string()
}
```

**What enters the hash:**
- `method` = "POST"
- `path` = **full path with provider prefix** = `/gemini/v1beta/models/gemini-2.5-flash:generateContent`
- `normalized_body` = request body with `stream` field stripped, keys sorted

**Critical:** The path **includes** the model name (as a URL segment), so the cache key **does discriminate by model**. But the model **name itself is not extracted** for metadata storage.

### 6. Provider Enum & Gemini Identification
**File:** `crates/lcp-core/src/provider.rs`, **lines 4–46**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Anthropic,
    OpenAi,
    OpenRouter,
    Gemini,  // Line 9
}

impl Provider {
    pub fn path_prefix(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",  // Line 19
            ...
        }
    }

    pub fn default_upstream(self) -> &'static str {
        match self {
            Provider::Gemini => "https://generativelanguage.googleapis.com",  // Line 29
            ...
        }
    }

    pub fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "gemini" => Some(Provider::Gemini),  // Line 39
            ...
        }
    }
}
```

### 7. What cache_key() Receives for a Gemini Request

**From proxy.rs, line 79:**
- `method` = `"POST"`
- `path` = `/gemini/v1beta/models/gemini-2.5-flash:generateContent`
- `body` = `{"contents":[{"parts":[{"text":"..."}]}]}`

**Normalized body (hash.rs, line 11):**
- Parse as JSON
- Strip `stream` field (none present in Gemini request)
- Sort keys (none in top-level object)
- Re-serialize

**Hash input:** `"POST|/gemini/v1beta/models/gemini-2.5-flash:generateContent|{\"contents\":[{\"parts\":[{\"text\":\"...\"}]}]}"`

### 8. ServerConfig Upstream Fields
**File:** `crates/lcp-server/src/server.rs`, **lines 13–25**

```rust
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub cache: Cache,
    pub timeout_seconds: u64,
    /// Override upstream URL per provider. Falls back to provider default when absent.
    pub anthropic_upstream: Option<String>,
    pub openai_upstream: Option<String>,
    pub openrouter_upstream: Option<String>,
    pub gemini_upstream: Option<String>,  // Line 22
    pub stream_channel_capacity: usize,
}
```

**Gemini upstream default:**
- **Defined in:** `crates/lcp-core/src/provider.rs`, line 29
- **Default value:** `"https://generativelanguage.googleapis.com"`
- **Env override:** `LCP_GEMINI_UPSTREAM` (per SPEC.md, line 41)
- **CLI override:** `--gemini-upstream` (per crates/lcp/src/main.rs, lines 46–48)

### 9. Existing Tests for Model Extraction
**File:** `tests/common/harness.rs`, **line 206** — Example request in harness tests:

```rust
.body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[]}"#)
```

**Test specs checked:**
- `tests/spec/cache_hit.rs`, `tests/spec/cache_miss.rs` — Both use Anthropic requests with `model` field in body
- **No tests for Gemini model extraction from URL path**
- **No tests that verify `model=NULL` for Gemini entries**

**Unit tests for cache_key() (hash.rs, lines 73–131):**
- `identical_bodies_same_key` — Lines 77–83
- `stream_field_stripped` — Lines 86–93
- `key_order_independent` — Lines 95–103
- `different_bodies_different_keys` — Lines 105–113
- `different_paths_different_keys` — Lines 115–122
- `invalid_json_hashes_raw` — Lines 124–130

None of these test Gemini-style paths or model extraction from URL.

### 10. Body Normalization Function
**File:** `crates/lcp-core/src/hash.rs`, **lines 27–34**

```rust
fn normalize_body(body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return String::from_utf8_lossy(body).into_owned();
    };
    strip_fields(&mut value, &["stream"]);
    sort_keys(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
}
```

**What normalize_body does:**
- **Does NOT strip `model` field** (per spec: "The model field MUST NOT be stripped" — SPEC.md, line 58)
- **Strips only `stream`** field (transport-level, not semantic)
- **Sorts all JSON object keys recursively**
- **For Gemini:** No `model` field exists to preserve or strip; normalization proceeds normally

---

## Architecture & Data Flow

```
Client Request
    ↓
Router (router.rs:16) → /{provider}/{*path} POST
    ↓
proxy::handle() (proxy.rs:57)
    ├─ Parse provider: Provider::from_prefix(provider_str) → Provider::Gemini
    ├─ Check x-lcp-bypass header
    ├─ Compute full_path: "/gemini/v1beta/models/gemini-2.5-flash:generateContent"
    ├─ Compute cache_key("POST", full_path, body) via hash::cache_key()
    │   └─ normalize_body() → sort keys, strip stream
    ├─ Cache lookup (if not bypassed)
    │   └─ MISS → continue to upstream
    │   └─ HIT → serve_cached(), return
    ├─ Extract model: extract_model(&body) → None (no model field in Gemini request)
    ├─ Build upstream URL:
    │   └─ upstream = "https://generativelanguage.googleapis.com"
    │   └─ url = upstream + "/" + path
    │   └─ Final: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    ├─ Forward request to upstream (POST)
    ├─ Stream response back to client
    └─ On 2xx status: cache::put(key, "gemini", None, exchange)
        ├─ Store in DB: provider="gemini", model=NULL
        └─ In stats(): SELECT model, COUNT(*) → (NULL, 1)
            └─ Convert NULL to "unknown" in by_model (cache.rs:190)
```

---

## Problem Statement (from FINDINGS.md)

**Issue:** Gemini requests encode the model in the URL path (e.g., `/v1beta/models/gemini-2.5-flash:generateContent`), not in the request body's `model` field.

**Current behavior:**
- `extract_model(&body)` returns `None` (no `model` field in Gemini body)
- `cache.put(..., model=None, ...)` stores `NULL` in the `model` column
- `cache.stats()` converts `NULL` to `"unknown"` (cache.rs:190)
- Stats shows: `{"by_model": {"unknown": 1}}`

**Expected behavior (desired fix):**
- Extract model name from URL path segment (e.g., `/models/gemini-2.5-flash:generateContent`)
- Store extracted model in cache: `"gemini-2.5-flash"`
- Stats shows: `{"by_model": {"gemini-2.5-flash": 1}}`

---

## Start Here

**Next step:** Create a spec document (or plan) that defines:
1. **URL path pattern parsing** — How to extract model from Gemini URLs (and other provider-specific patterns if any)
2. **Provider-specific model extraction** — Either a provider enum method or a per-provider path regex/function
3. **Backward compatibility** — Ensure non-Gemini providers (body-based) continue to work
4. **Cache key stability** — Confirm that adding model extraction does not change cache keys (it shouldn't, since path is already in the key)

**Implementation entry points:**
- `crates/lcp-server/src/proxy.rs` — Modify `extract_model()` or create a new function that:
  - Calls provider-specific extraction
  - Falls back to body extraction if provider doesn't support path-based models
- Potentially extend `Provider` enum in `crates/lcp-core/src/provider.rs` with a method like `extract_model_from_path(path: &str) -> Option<String>`
