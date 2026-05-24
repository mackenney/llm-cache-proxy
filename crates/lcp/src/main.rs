use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use lcp_core::Cache;
use lcp_server::{ServerConfig, serve};

#[derive(Parser, Debug)]
#[command(name = "lcp", about = "Local LLM API caching proxy")]
struct Cli {
    /// Port to listen on.
    #[arg(long, env = "LCP_PORT", default_value = "9001")]
    port: u16,

    /// Host to bind to.
    #[arg(long, env = "LCP_HOST", default_value = "127.0.0.1")]
    host: String,

    /// Path to the SQLite cache database.
    #[arg(long, env = "LCP_DB")]
    db: Option<PathBuf>,

    /// Cache TTL in seconds. 0 means entries never expire.
    #[arg(long, env = "LCP_TTL", default_value = "0")]
    ttl: u64,

    /// Upstream request timeout in seconds. 0 means no timeout.
    #[arg(long, env = "LCP_TIMEOUT", default_value = "300")]
    timeout: u64,

    /// Override the Anthropic upstream URL.
    #[arg(long, env = "LCP_ANTHROPIC_UPSTREAM")]
    anthropic_upstream: Option<String>,

    /// Override the OpenAI upstream URL.
    #[arg(long, env = "LCP_OPENAI_UPSTREAM")]
    openai_upstream: Option<String>,

    /// Override the OpenRouter upstream URL.
    #[arg(long, env = "LCP_OPENROUTER_UPSTREAM")]
    openrouter_upstream: Option<String>,

    /// Override the Gemini upstream URL.
    #[arg(long, env = "LCP_GEMINI_UPSTREAM")]
    gemini_upstream: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("lcp=info".parse()?))
        .init();

    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(default_db_path);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cache = Cache::open(&db_path, cli.ttl)?;
    tracing::info!(path = %db_path.display(), "cache database opened");

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;

    let config = ServerConfig {
        addr,
        cache,
        timeout_seconds: cli.timeout,
        anthropic_upstream: cli.anthropic_upstream,
        openai_upstream: cli.openai_upstream,
        openrouter_upstream: cli.openrouter_upstream,
        gemini_upstream: cli.gemini_upstream,
    };

    tracing::info!("set ANTHROPIC_BASE_URL=http://{addr}/anthropic");
    tracing::info!("set OPENAI_BASE_URL=http://{addr}/openai");
    tracing::info!("set OPENROUTER_BASE_URL=http://{addr}/openrouter");
    tracing::info!("set GEMINI_BASE_URL=http://{addr}/gemini");

    serve(config).await
}

fn default_db_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lcp")
        .join("cache.db")
}
