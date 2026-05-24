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
    pub method: String,
    pub path: String,
    pub body: String,
}

/// A complete request/response exchange stored in the cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    pub request: RequestRecord,
    pub status: u16,
    pub content_type: String,
    /// For SSE responses: ordered chunks. For JSON responses: a single chunk.
    pub chunks: Vec<ResponseChunk>,
}

/// A row from the cache DB (metadata only, no chunks inline).
#[derive(Debug)]
pub struct CacheEntry {
    pub key: String,
    pub created_at: String, // ISO-8601 UTC
    pub provider: String,
    pub model: Option<String>,
    pub status: u16,
    pub hit_count: i64,
    pub req_bytes: i64,
    pub resp_bytes: i64,
}
