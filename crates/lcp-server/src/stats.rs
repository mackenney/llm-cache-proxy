use axum::Json;
use axum::extract::{Path, State};
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
    match state.config.cache.stats() {
        Ok(s) => Json(serde_json::to_value(s).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn clear_stats(State(state): State<AppState>) -> impl IntoResponse {
    match state.config.cache.clear_stats() {
        Ok(()) => Json(json!({"cleared": true})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn clear_cache(State(state): State<AppState>) -> impl IntoResponse {
    match state.config.cache.clear_entries() {
        Ok(n) => Json(json!({"cleared_entries": n})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn get_trace(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.config.cache.get_trace(&trace_id) {
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
