use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;

use lcp_core::{
    Provider, cache_key,
    types::{Exchange, RequestRecord, ResponseChunk},
};

use crate::server::ServerConfig;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub client: Arc<reqwest::Client>,
}

pub async fn handle(
    State(state): State<AppState>,
    Path((provider_str, path)): Path<(String, String)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(provider) = Provider::from_prefix(&provider_str) else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown provider: {provider_str}"),
        )
            .into_response();
    };

    let bypass = headers.get("x-lcp-bypass").and_then(|v| v.to_str().ok()) == Some("1");
    let trace_id = headers
        .get("x-lcp-trace")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let full_path = format!("/{provider_str}/{path}");
    let key = cache_key("POST", &full_path, &body);

    if !bypass {
        match state.config.cache.get(&key) {
            Ok(Some(exchange)) => {
                if let Some(ref tid) = trace_id {
                    if let Err(e) = state.config.cache.record_trace(tid, &key) {
                        tracing::warn!(err = %e, "failed to record trace on hit");
                    }
                }
                return serve_cached(exchange, &key);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(err = %e, "cache lookup failed, falling through to upstream");
            }
        }
    }

    let upstream = state.config.upstream_for(provider);
    let mut url = format!("{}/{}", upstream.trim_end_matches('/'), path);
    if !query.is_empty() {
        let qs = query
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&qs);
    }

    let mut rb = state.client.post(&url).body(body.clone());
    for (name, value) in &headers {
        let n = name.as_str();
        if matches!(
            n,
            "host" | "connection" | "transfer-encoding" | "accept-encoding" | "content-length"
        ) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            rb = rb.header(name.clone(), v);
        }
    }

    let upstream_resp = match rb.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(err = %e, url = %url, "upstream request failed");
            return (StatusCode::BAD_GATEWAY, e.to_string()).into_response();
        }
    };

    let status = upstream_resp.status();
    let content_type = upstream_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let is_sse = content_type.contains("text/event-stream");

    let model = extract_model(&body);
    let start = Instant::now();
    let mut chunks: Vec<ResponseChunk> = Vec::new();
    let mut full_body: Vec<u8> = Vec::new();

    let mut stream = upstream_resp.bytes_stream();
    while let Some(result) = stream.next().await {
        match result {
            Ok(bytes) => {
                let offset_ms = start.elapsed().as_millis() as u64;
                chunks.push(ResponseChunk {
                    offset_ms,
                    data: String::from_utf8_lossy(&bytes).into_owned(),
                });
                full_body.extend_from_slice(&bytes);
            }
            Err(e) => {
                tracing::warn!(err = %e, "upstream stream error");
                break;
            }
        }
    }

    if !bypass && status.is_success() {
        let exchange = Exchange {
            request: RequestRecord {
                method: "POST".into(),
                path: full_path,
                body: String::from_utf8_lossy(&body).into_owned(),
            },
            status: status.as_u16(),
            content_type: content_type.clone(),
            chunks,
        };
        match state
            .config
            .cache
            .put(&key, provider.path_prefix(), model.as_deref(), &exchange)
        {
            Ok(()) => {
                if let Some(ref tid) = trace_id {
                    if let Err(e) = state.config.cache.record_trace(tid, &key) {
                        tracing::warn!(err = %e, "failed to record trace on miss");
                    }
                }
            }
            Err(e) => tracing::warn!(err = %e, "failed to cache exchange"),
        }
    }

    let lcp_status = if bypass { "BYPASS" } else { "MISS" };
    let mut response = Response::builder()
        .status(status)
        .header("content-type", &content_type)
        .header("x-lcp-cache", lcp_status);

    if !bypass {
        response = response.header("x-lcp-key", &key[..12]);
    }

    if is_sse {
        response = response
            .header("cache-control", "no-cache")
            .header("transfer-encoding", "chunked");
    }

    response
        .body(Body::from(full_body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn serve_cached(exchange: Exchange, key: &str) -> Response {
    let body: Vec<u8> = exchange
        .chunks
        .into_iter()
        .flat_map(|c| c.data.into_bytes())
        .collect();

    Response::builder()
        .status(exchange.status)
        .header("content-type", &exchange.content_type)
        .header("x-lcp-cache", "HIT")
        .header("x-lcp-key", &key[..12])
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned))
}
