use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;

use lcp_core::Cache;

use crate::extensions::ExtensionPipeline;
use crate::proxy::AppState;
use crate::router::build_router;

/// Runtime configuration for the proxy server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub cache: Cache,
    /// Upstream request timeout in seconds. 0 means no timeout.
    pub timeout_seconds: u64,
    /// Override upstream URL per provider. Falls back to provider default when absent.
    pub anthropic_upstream: Option<String>,
    pub openai_upstream: Option<String>,
    pub openrouter_upstream: Option<String>,
    pub gemini_upstream: Option<String>,
    /// Bounded channel capacity for streaming response chunks. Default: 32.
    pub stream_channel_capacity: usize,
    /// Extension pipeline applied to every proxied request.
    pub extensions: ExtensionPipeline,
}

impl ServerConfig {
    pub fn upstream_for(&self, provider: lcp_core::Provider) -> String {
        match provider {
            lcp_core::Provider::Anthropic => self
                .anthropic_upstream
                .clone()
                .unwrap_or_else(|| provider.default_upstream().to_owned()),
            lcp_core::Provider::OpenAi => self
                .openai_upstream
                .clone()
                .unwrap_or_else(|| provider.default_upstream().to_owned()),
            lcp_core::Provider::OpenRouter => self
                .openrouter_upstream
                .clone()
                .unwrap_or_else(|| provider.default_upstream().to_owned()),
            lcp_core::Provider::Gemini => self
                .gemini_upstream
                .clone()
                .unwrap_or_else(|| provider.default_upstream().to_owned()),
        }
    }
}

/// Build an HTTP client for upstream requests.
///
/// Compression negotiation is disabled so upstreams return plain SSE.
pub fn build_upstream_client(timeout_seconds: u64) -> reqwest::Client {
    let mut cb = reqwest::Client::builder()
        // Never negotiate compression — upstreams must return plain SSE.
        .no_gzip()
        .no_deflate()
        .no_brotli();
    if timeout_seconds > 0 {
        cb = cb.timeout(std::time::Duration::from_secs(timeout_seconds));
    }
    cb.build().expect("build reqwest Client")
}

/// Start the proxy and block until the server terminates.
pub async fn serve(config: ServerConfig) -> Result<()> {
    let addr = config.addr;
    let timeout_seconds = config.timeout_seconds;

    let client = Arc::new(build_upstream_client(timeout_seconds));

    let state = AppState {
        config: Arc::new(config),
        client,
        background_writes: Arc::new(tokio::sync::Mutex::new(tokio::task::JoinSet::new())),
    };

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %listener.local_addr()?, "lcp proxy listening");
    axum::serve(listener, app).await?;
    Ok(())
}
