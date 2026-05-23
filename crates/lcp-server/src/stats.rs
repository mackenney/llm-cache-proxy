use axum::Json;
use axum::extract::State;
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
