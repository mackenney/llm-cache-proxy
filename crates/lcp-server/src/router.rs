use axum::Router;
use axum::routing::{delete, get, post};
use tower_http::decompression::RequestDecompressionLayer;

use crate::proxy::AppState;
use crate::{proxy, stats};

/// Build the Axum [`Router`] for the proxy server.
///
/// Mounts the admin endpoints (`/`, `/stats`, `/cache`, `/trace`) and the
/// catch-all proxy handler at `/{provider}/{*path}`. Applies request
/// decompression so upstreams always receive plain bodies.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(stats::health))
        .route("/stats", get(stats::get_stats))
        .route("/stats", delete(stats::clear_stats))
        .route("/cache", delete(stats::clear_cache))
        .route("/cache/{key}", get(stats::get_cache_entry))
        .route("/trace/{trace_id}", get(stats::get_trace))
        .route("/{provider}/{*path}", post(proxy::handle))
        .route("/{provider}/{*path}", get(proxy::handle))
        .layer(RequestDecompressionLayer::new())
        .with_state(state)
}
