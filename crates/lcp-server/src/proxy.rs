use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;

use lcp_core::{
    Provider, cache_key,
    types::{Exchange, RequestRecord, ResponseChunk},
};

use crate::extensions::{ProxyCtx, ResponseStream};
use crate::server::ServerConfig;

/// Adapts `mpsc::Receiver<T>` to `Stream<Item = T>` for use with `Body::from_stream()`.
/// Avoids adding tokio-stream dependency for this single use case.
struct ReceiverStream<T> {
    rx: mpsc::Receiver<T>,
}

impl<T> ReceiverStream<T> {
    fn new(rx: mpsc::Receiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    // Receiver<T>: Unpin (Arc-backed), so Pin<&mut Self> auto-derefs to &mut Self.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServerConfig>,
    pub client: Arc<reqwest::Client>,
    pub background_writes: Arc<Mutex<JoinSet<()>>>,
}

impl AppState {
    pub async fn wait_for_pending_writes(&self) {
        let mut set = self.background_writes.lock().await;
        while set.join_next().await.is_some() {}
    }
}

pub async fn handle(
    State(state): State<AppState>,
    method: Method,
    Path((provider_str, path)): Path<(String, String)>,
    original_uri: OriginalUri,
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

    let ctx = ProxyCtx {
        provider,
        method: method.to_string(),
        path: path.clone(),
        cache_key: None,
    };

    // Phase 1: transform body before cache key (fires on every proxied request).
    let body = match state.config.extensions.run_phase1(ctx.clone(), body).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(err = %e, "extension phase 1 error");
            return (StatusCode::INTERNAL_SERVER_ERROR, "extension error").into_response();
        }
    };

    let key = cache_key(provider, method.as_str(), &full_path, &body);
    let ctx = ProxyCtx {
        cache_key: Some(key.clone()),
        ..ctx
    };

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

    // Phase 2: transform body for the wire (cache miss path only, not bypass).
    let (wire_body, ext_states) = if !bypass {
        match state
            .config
            .extensions
            .run_phase2(ctx.clone(), body.clone())
            .await
        {
            Ok(x) => x,
            Err(e) => {
                tracing::error!(err = %e, "extension phase 2 error");
                return (StatusCode::INTERNAL_SERVER_ERROR, "extension error").into_response();
            }
        }
    } else {
        (body.clone(), Vec::new())
    };

    let upstream = state.config.upstream_for(provider);
    let mut url = format!("{}/{}", upstream.trim_end_matches('/'), path);
    if let Some(query) = original_uri.query() {
        url.push('?');
        url.push_str(query);
    }

    let mut rb = state.client.request(method.clone(), &url).body(wire_body);
    for (name, value) in &headers {
        let n = name.as_str();
        if matches!(
            n,
            "host"
                | "connection"
                | "transfer-encoding"
                | "accept-encoding"
                | "content-encoding"
                | "content-length"
                | "x-lcp-bypass"
                | "x-lcp-trace"
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

    let model = provider
        .extract_model_from_path(&path)
        .or_else(|| extract_model(&body));
    let do_cache = !bypass && status.is_success();

    // Create channel for streaming response to client
    // Bounded channel; capacity bounds in-flight chunks and provides backpressure.
    let (tx, rx) =
        mpsc::channel::<Result<Bytes, std::io::Error>>(state.config.stream_channel_capacity);

    // Clone values needed by the spawned task
    let cache = state.config.cache.clone();
    let key_clone = key.clone();
    let full_path_clone = full_path.clone();
    let body_clone = body.clone();
    let trace_id_clone = trace_id.clone();
    let provider_prefix = provider.path_prefix().to_owned();
    let model_clone = model.clone();
    let content_type_clone = content_type.clone();
    let status_code = status.as_u16();
    let method_clone = method.to_string();

    // Phase 3: wrap the upstream stream (cache miss path only, not bypass).
    // Constructed before spawning so extension state is captured inside the stream.
    let raw_stream: ResponseStream = Box::pin(
        upstream_resp
            .bytes_stream()
            .map(|r| r.map_err(|e| std::io::Error::other(e.to_string()))),
    );
    let response_stream = if !bypass {
        state
            .config
            .extensions
            .run_phase3(ctx, ext_states, raw_stream)
    } else {
        raw_stream
    };

    // Spawn task to read upstream, forward to client channel, and cache on completion.
    // Reap any already-completed tasks to prevent unbounded handle accumulation.
    {
        let mut set = state.background_writes.lock().await;
        set.spawn(async move {
            let mut chunks: Vec<ResponseChunk> = Vec::new();
            let mut stream = response_stream;
            let start = Instant::now();

            let stream_complete = loop {
                match stream.next().await {
                    Some(Ok(bytes)) => {
                        // Only accumulate for cache write; skip string conversion when not caching.
                        if do_cache {
                            let offset_ms = start.elapsed().as_millis() as u64;
                            chunks.push(ResponseChunk {
                                offset_ms,
                                data: String::from_utf8_lossy(&bytes).into_owned(),
                            });
                        }
                        // Forward to client; break if client disconnected
                        if tx.send(Ok(bytes)).await.is_err() {
                            tracing::debug!("client disconnected mid-stream");
                            break false; // Don't cache partial responses
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(err = %e, chunks = chunks.len(), "upstream stream error");
                        let _ = tx.send(Err(e)).await;
                        break false; // Don't cache on error
                    }
                    None => break true, // Stream completed successfully
                }
            };
            drop(tx); // Signal end-of-stream to receiver

            // Cache write only if stream completed successfully and caching is enabled
            if stream_complete && do_cache {
                let exchange = Exchange {
                    request: RequestRecord {
                        method: method_clone.clone(),
                        path: full_path_clone,
                        body: String::from_utf8_lossy(&body_clone).into_owned(),
                    },
                    status: status_code,
                    content_type: content_type_clone,
                    chunks,
                };
                match cache.put(
                    &key_clone,
                    &provider_prefix,
                    model_clone.as_deref(),
                    &exchange,
                ) {
                    Ok(()) => {
                        if let Some(ref tid) = trace_id_clone {
                            if let Err(e) = cache.record_trace(tid, &key_clone) {
                                tracing::warn!(err = %e, "failed to record trace on miss");
                            }
                        }
                    }
                    Err(e) => tracing::warn!(err = %e, "failed to cache exchange"),
                }
            }
        });
        // Reap completed tasks without blocking; prevents unbounded handle accumulation.
        while set.try_join_next().is_some() {}
    }

    // Build streaming response
    let body_stream = ReceiverStream::new(rx);
    let body = Body::from_stream(body_stream);

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
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn serve_cached(exchange: Exchange, key: &str) -> Response {
    // Stream chunks directly — preserves original chunk boundaries
    let chunk_stream = futures_util::stream::iter(
        exchange
            .chunks
            .into_iter()
            .map(|c| Ok::<_, std::io::Error>(Bytes::from(c.data))),
    );
    let body = Body::from_stream(chunk_stream);

    Response::builder()
        .status(exchange.status)
        .header("content-type", &exchange.content_type)
        .header("x-lcp-cache", "HIT")
        .header("x-lcp-key", &key[..12])
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned))
}
