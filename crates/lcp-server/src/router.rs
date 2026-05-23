use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::proxy::AppState;
use crate::server::ServerConfig;
use crate::{proxy, stats};

pub fn build_router(config: Arc<ServerConfig>, client: Arc<reqwest::Client>) -> Router {
    let state = AppState { config, client };

    Router::new()
        .route("/", get(stats::health))
        .route("/stats", get(stats::get_stats))
        .route("/stats", delete(stats::clear_stats))
        .route("/cache", delete(stats::clear_cache))
        .route("/{provider}/{*path}", post(proxy::handle))
        .route("/{provider}/{*path}", get(proxy::handle))
        .with_state(state)
}
