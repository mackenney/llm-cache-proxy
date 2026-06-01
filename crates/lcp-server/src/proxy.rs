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
    Provider, cache_key_and_model,
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

    let (key, model_from_body) = cache_key_and_model(provider, method.as_str(), &full_path, &body);
    let ctx = ProxyCtx {
        cache_key: Some(key.clone()),
        ..ctx
    };

    if !bypass {
        let cache = state.config.cache.clone();
        let key_for_lookup = key.clone();
        let cached = tokio::task::spawn_blocking(move || cache.get(&key_for_lookup))
            .await
            .expect("spawn_blocking panicked");
        match cached {
            Ok(Some(exchange)) => {
                if let Some(ref tid) = trace_id {
                    let cache = state.config.cache.clone();
                    let tid = tid.clone();
                    let key_for_trace = key.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        cache.record_trace(&tid, &key_for_trace)
                    })
                    .await
                    .expect("spawn_blocking panicked")
                    {
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
        if n.starts_with("x-lcp-")
            || matches!(
                n,
                "host"
                    | "connection"
                    | "transfer-encoding"
                    | "accept-encoding"
                    | "content-encoding"
                    | "content-length"
            )
        {
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

    let model = provider.extract_model_from_path(&path).or(model_from_body);
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
            let mut chunks_raw: Vec<(u64, Bytes)> = Vec::new();
            let mut stream = response_stream;
            // Accumulate raw response bytes to validate UTF-8 after the full stream.
            // Per-chunk checks fail on valid multibyte sequences split across chunk boundaries.
            let mut response_buf: Vec<u8> = Vec::new();
            let start = Instant::now();

            let stream_complete = loop {
                match stream.next().await {
                    Some(Ok(bytes)) => {
                        // Only accumulate for cache write; skip string conversion when not caching.
                        if do_cache {
                            let offset_ms = start.elapsed().as_millis() as u64;
                            response_buf.extend_from_slice(&bytes);
                            chunks_raw.push((offset_ms, bytes.clone()));
                        }
                        // Forward to client; break if client disconnected
                        if tx.send(Ok(bytes)).await.is_err() {
                            tracing::debug!("client disconnected mid-stream");
                            break false; // Don't cache partial responses
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(err = %e, chunks = chunks_raw.len(), "upstream stream error");
                        let _ = tx.send(Err(e)).await;
                        break false; // Don't cache on error
                    }
                    None => break true, // Stream completed successfully
                }
            };
            drop(stream); // Drop stream here to satisfy SensitiveState lifetime guarantee.
            drop(tx); // Signal end-of-stream to receiver.

            // Skip caching if response or request body contains non-UTF8 bytes.
            // Validate after full stream accumulation so multibyte sequences split across
            // chunk boundaries are handled correctly.
            let response_is_valid_utf8 = std::str::from_utf8(&response_buf).is_ok();
            let request_is_valid_utf8 = std::str::from_utf8(&body_clone).is_ok();
            if do_cache && (!response_is_valid_utf8 || !request_is_valid_utf8) {
                if !response_is_valid_utf8 {
                    tracing::warn!(key = %key_clone, "skipping cache: response stream contains non-UTF8 bytes");
                } else {
                    tracing::warn!(key = %key_clone, "skipping cache: request body contains non-UTF8 bytes");
                }
            }
            if stream_complete && do_cache && response_is_valid_utf8 && request_is_valid_utf8 {
                let chunks: Vec<ResponseChunk> = {
                    let mut carry: Vec<u8> = Vec::new();
                    chunks_raw
                        .into_iter()
                        .map(|(offset_ms, raw)| {
                            let mut buf = std::mem::take(&mut carry);
                            buf.extend_from_slice(&raw);
                            match std::str::from_utf8(&buf) {
                                Ok(s) => ResponseChunk {
                                    offset_ms,
                                    data: s.to_owned(),
                                },
                                Err(e) => {
                                    let valid_up_to = e.valid_up_to();
                                    let data = std::str::from_utf8(&buf[..valid_up_to])
                                        .expect("valid_up_to is a char boundary")
                                        .to_owned();
                                    carry = buf[valid_up_to..].to_vec();
                                    ResponseChunk { offset_ms, data }
                                }
                            }
                        })
                        .collect()
                };
                let exchange = Exchange {
                    request: RequestRecord {
                        method: method_clone,
                        path: full_path_clone,
                        body: String::from_utf8_lossy(&body_clone).into_owned(),
                    },
                    status: status_code,
                    content_type: content_type_clone,
                    chunks,
                };
                let put_result = tokio::task::spawn_blocking({
                    let cache = cache.clone();
                    let key = key_clone.clone();
                    let provider = provider_prefix.clone();
                    let model = model_clone.clone();
                    move || cache.put(&key, &provider, model.as_deref(), &exchange)
                })
                .await
                .expect("spawn_blocking panicked");
                match put_result {
                    Ok(()) => {
                        if let Some(ref tid) = trace_id_clone {
                            let trace_result = tokio::task::spawn_blocking({
                                let cache = cache.clone();
                                let tid = tid.clone();
                                let key = key_clone.clone();
                                move || cache.record_trace(&tid, &key)
                            })
                            .await
                            .expect("spawn_blocking panicked");
                            if let Err(e) = trace_result {
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
