use serde::{Deserialize, Serialize};

/// A single recorded SSE chunk with its arrival offset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseChunk {
    /// Milliseconds since the first chunk arrived.
    pub offset_ms: u64,
    /// Raw bytes of the chunk (UTF-8 SSE data or JSON body).
    pub data: String,
}

/// The upstream request as recorded at cache-miss time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestRecord {
    /// HTTP method (`GET`, `POST`, …).
    pub method: String,
    /// Request path including provider prefix, e.g. `/anthropic/v1/messages`.
    pub path: String,
    /// Raw request body as a UTF-8 string.
    pub body: String,
}

/// A complete request/response exchange stored in the cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    /// The original request.
    pub request: RequestRecord,
    /// HTTP status code of the upstream response.
    pub status: u16,
    /// Value of the upstream `Content-Type` response header.
    pub content_type: String,
    /// For SSE responses: ordered chunks. For JSON responses: a single chunk.
    pub chunks: Vec<ResponseChunk>,
}

/// A row from the cache DB — metadata only, no chunks inline.
#[derive(Debug)]
pub struct CacheEntry {
    /// BLAKE3 hex digest that is the cache key.
    pub key: String,
    /// ISO-8601 UTC timestamp of when the entry was stored.
    pub created_at: String,
    /// Provider name (e.g. `anthropic`, `openai`).
    pub provider: String,
    /// Model name extracted from the request, if available.
    pub model: Option<String>,
    /// HTTP status code of the stored response.
    pub status: u16,
    /// Number of times this entry has been served from cache.
    pub hit_count: i64,
    /// Size of the stored request body in bytes.
    pub req_bytes: i64,
    /// Size of the stored response body in bytes.
    pub resp_bytes: i64,
}

/// A complete cache row: metadata plus the stored request and response.
#[derive(Debug, Clone, Serialize)]
pub struct FullEntry {
    /// BLAKE3 hex digest that is the cache key.
    pub key: String,
    /// ISO-8601 UTC timestamp of when the entry was stored.
    pub created_at: String,
    /// Provider name (e.g. `anthropic`, `openai`).
    pub provider: String,
    /// Model name extracted from the request, if available.
    pub model: Option<String>,
    /// HTTP status code of the stored response.
    pub status: u16,
    /// Response `Content-Type` header value.
    pub content_type: String,
    /// Number of times this entry has been served from cache.
    pub hit_count: i64,
    /// Size of the stored request body in bytes.
    pub req_bytes: i64,
    /// Size of the stored response body in bytes.
    pub resp_bytes: i64,
    /// The original request.
    pub request: RequestRecord,
    /// Ordered response chunks (one per SSE event, or one chunk for JSON).
    pub chunks: Vec<ResponseChunk>,
}
