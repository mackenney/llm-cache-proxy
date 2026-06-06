use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use tower_http::decompression::RequestDecompressionLayer;

use crate::proxy::AppState;
use crate::{proxy, stats};

/// Build the Axum [`Router`] for the proxy server.
///
/// Proxy routes get a configurable body limit (from `state.config.body_limit_bytes`).
/// Admin routes keep Axum's default 2 MiB limit since they never receive large bodies.
/// `RequestDecompressionLayer` wraps the merged router so it runs on all routes.
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.body_limit_bytes;

    let proxy_routes = Router::new()
        .route("/{provider}/{*path}", post(proxy::handle))
        .route("/{provider}/{*path}", get(proxy::handle));

    let proxy_routes = if body_limit == 0 {
        proxy_routes.layer(DefaultBodyLimit::disable())
    } else {
        proxy_routes.layer(DefaultBodyLimit::max(body_limit as usize))
    };

    Router::new()
        .route("/", get(stats::health))
        .route("/stats", get(stats::get_stats))
        .route("/stats", delete(stats::clear_stats))
        .route("/cache", delete(stats::clear_cache))
        .route("/cache/{key}", get(stats::get_cache_entry))
        .route("/trace/{trace_id}", get(stats::get_trace))
        .merge(proxy_routes)
        .layer(RequestDecompressionLayer::new())
        .with_state(state)
}
