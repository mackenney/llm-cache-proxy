//! Cassette loader: reads fixture TOML files into MockResponse sequences.
//!
//! Cassettes store real provider SSE responses captured with synthetic test keys.
//! The fake key embedded in body_chunks is the doppel substitution of the synthetic
//! secret; the test infrastructure restores it during replay via the proxy.
#![allow(dead_code)]

use std::path::Path;

use bytes::Bytes;
use serde::Deserialize;

/// A loaded cassette ready to feed into MockUpstreamBuilder.
pub struct Cassette {
    /// Provider that produced this recording.
    pub provider: String,
    /// Human-readable scenario tag.
    pub scenario: String,
    /// Which secret kind was used (drives doppel pattern selection on replay).
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
    ///
    /// Path is relative to the tests package directory (e.g. "fixtures/anthropic/dummy.toml").
    /// Panics with a clear message on missing file or malformed TOML — callers are tests.
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("Cassette::load: cannot read {}: {}", path.display(), e));
        let raw: CassetteFile = toml::from_str(&text).unwrap_or_else(|e| {
            panic!(
                "Cassette::load: malformed TOML in {}: {}",
                path.display(),
                e
            )
        });

        let entry = raw.response.into_iter().next().unwrap_or_else(|| {
            panic!(
                "Cassette::load: no [[response]] entries in {}",
                path.display()
            )
        });

        // Strip hop-by-hop headers that axum manages itself and that cannot be
        // set explicitly on a streaming response builder.
        const HOP_BY_HOP: &[&str] = &[
            "transfer-encoding",
            "connection",
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "te",
            "trailers",
            "upgrade",
        ];
        let headers: Vec<(String, String)> = entry
            .headers
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| {
                if HOP_BY_HOP.iter().any(|h| k.to_lowercase() == *h) {
                    return None;
                }
                let vs = match v {
                    toml::Value::String(s) => s,
                    other => other.to_string(),
                };
                Some((k, vs))
            })
            .collect();

        let body_chunks: Vec<Bytes> = entry
            .body_chunks
            .into_iter()
            .map(|s| Bytes::from(s.into_bytes()))
            .collect();

        Cassette {
            provider: raw.provider,
            scenario: raw.scenario,
            secret_kind: raw.secret_kind,
            status: entry.status,
            headers,
            body_chunks,
        }
    }

    /// All chunks concatenated — for non-SSE (JSON) cassettes or assertions.
    pub fn full_body(&self) -> Bytes {
        let total: usize = self.body_chunks.iter().map(|b| b.len()).sum();
        let mut out = bytes::BytesMut::with_capacity(total);
        for chunk in &self.body_chunks {
            out.extend_from_slice(chunk);
        }
        out.freeze()
    }
}

#[derive(Deserialize)]
struct CassetteFile {
    provider: String,
    scenario: String,
    secret_kind: String,
    response: Vec<ResponseEntry>,
}

#[derive(Deserialize)]
struct ResponseEntry {
    status: u16,
    #[serde(default)]
    headers: Option<toml::Table>,
    #[serde(default)]
    body_chunks: Vec<String>,
}
