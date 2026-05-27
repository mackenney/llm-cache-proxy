use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::proxy::AppState;

pub async fn health(_state: State<AppState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "lcp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn get_stats(State(state): State<AppState>) -> impl IntoResponse {
    let cache = state.config.cache.clone();
    let result = tokio::task::spawn_blocking(move || cache.stats())
        .await
        .expect("spawn_blocking panicked");
    match result {
        Ok(s) => match serde_json::to_value(s) {
            Ok(v) => Json(v).into_response(),
            Err(e) => {
                tracing::error!(err = %e, "failed to serialize stats");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn clear_stats(State(state): State<AppState>) -> impl IntoResponse {
    let cache = state.config.cache.clone();
    let result = tokio::task::spawn_blocking(move || cache.clear_stats())
        .await
        .expect("spawn_blocking panicked");
    match result {
        Ok(()) => Json(json!({"cleared": true})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn clear_cache(State(state): State<AppState>) -> impl IntoResponse {
    let cache = state.config.cache.clone();
    let result = tokio::task::spawn_blocking(move || cache.clear_entries())
        .await
        .expect("spawn_blocking panicked");
    match result {
        Ok(n) => Json(json!({"cleared_entries": n})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_cache_entry(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let cache = state.config.cache.clone();
    let key_clone = key.clone();
    let result = tokio::task::spawn_blocking(move || cache.inspect(&key_clone))
        .await
        .expect("spawn_blocking panicked");
    match result {
        Ok(Some(entry)) => match serde_json::to_value(&entry) {
            Ok(v) => Json(v).into_response(),
            Err(e) => {
                tracing::error!(err = %e, "failed to serialize cache entry");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Ok(None) => (StatusCode::NOT_FOUND, format!("cache key not found: {key}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TraceQuery {
    #[serde(default)]
    pub full: bool,
}

pub async fn get_trace(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
    Query(params): Query<TraceQuery>,
) -> impl IntoResponse {
    if params.full {
        let cache = state.config.cache.clone();
        let tid = trace_id.clone();
        let result = tokio::task::spawn_blocking(move || cache.inspect_trace(&tid))
            .await
            .expect("spawn_blocking panicked");
        match result {
            Ok(entries) => {
                let items: Vec<_> = entries
                    .iter()
                    .map(|e| serde_json::to_value(e).expect("FullEntry serializes"))
                    .collect();
                Json(json!({"trace_id": trace_id, "entries": items})).into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    } else {
        let cache = state.config.cache.clone();
        let tid = trace_id.clone();
        let result = tokio::task::spawn_blocking(move || cache.get_trace(&tid))
            .await
            .expect("spawn_blocking panicked");
        match result {
            Ok(entries) => {
                let items: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "key": e.key,
                            "created_at": e.created_at,
                            "status": e.status,
                            "hit_count": e.hit_count,
                        })
                    })
                    .collect();
                Json(json!({"trace_id": trace_id, "entries": items})).into_response()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}
