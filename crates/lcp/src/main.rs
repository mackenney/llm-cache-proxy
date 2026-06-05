use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

use lcp_core::Cache;
use lcp_server::{DoppelExt, ExtensionPipeline, ServerConfig, serve};

#[derive(Parser, Debug)]
#[command(
    name = "lcp",
    about = "Local HTTP proxy that caches LLM API calls. Point clients at http://127.0.0.1:9001/<provider>.",
    long_about = "lcp is a local HTTP proxy that caches LLM API responses on disk and replays them on subsequent identical requests, eliminating redundant API spend during iterative development.\n\nPoint your LLM client at lcp instead of the real API:\n  ANTHROPIC_BASE_URL=http://127.0.0.1:9001/anthropic\n  OPENAI_BASE_URL=http://127.0.0.1:9001/openai\n  OPENROUTER_BASE_URL=http://127.0.0.1:9001/openrouter\n  GEMINI_BASE_URL=http://127.0.0.1:9001/gemini\n\nFirst call goes to the real API and is cached. Subsequent identical calls are served from disk at full speed. Send x-lcp-bypass: 1 to skip the cache for a request.\n\nTag any request with x-lcp-trace: <id> to group it into a named trace session. Retrieve the full exchange log later with GET /trace/<id>."
)]
struct Cli {
    /// Path to config file (TOML). Defaults to $XDG_CONFIG_HOME/lcp/config.toml.
    #[arg(long, env = "LCP_CONFIG")]
    config: Option<PathBuf>,

    /// Print effective configuration as TOML and exit.
    #[arg(long)]
    print_config: bool,

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

/// Subset of config that can be set via the TOML config file.
/// Keys match long flag names with hyphens replaced by underscores.
#[derive(Deserialize, Default)]
struct FileConfig {
    port: Option<u16>,
    host: Option<String>,
    db: Option<String>,
    ttl: Option<u64>,
    timeout: Option<u64>,
    anthropic_upstream: Option<String>,
    openai_upstream: Option<String>,
    openrouter_upstream: Option<String>,
    gemini_upstream: Option<String>,
    extensions: Option<ExtensionsConfig>,
}

/// Per-extension configuration block under `[extensions]`.
#[derive(Deserialize, Default)]
struct ExtensionsConfig {
    doppel: Option<DoppelConfig>,
}

/// Configuration for the built-in swap/restore extension.
///
/// ```toml
/// [extensions.doppel]
/// secrets_file = "~/.config/lcp/secrets.toml"
/// ```
///
/// Create a secrets file with `doppel init --patterns <path>`.
/// Register additional secrets with `doppel register --patterns <path> --label <label>`.
#[derive(Deserialize, Default)]
struct DoppelConfig {
    secrets_file: Option<String>,
}

/// Scan argv for `--config <path>` or `--config=<path>` without a full parse.
/// Used before the tokio runtime starts so env-var seeding is single-threaded.
fn config_path_from_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    // Also honour LCP_CONFIG env var at this stage.
    if let Ok(v) = std::env::var("LCP_CONFIG") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            return iter.next().map(PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lcp")
        .join("config.toml")
}

/// Read the config file, pre-seed any `LCP_*` env vars that are not already
/// set, and return the parsed config for callers that need it.
///
/// Returns `None` if the file is missing (silently) or malformed (with a
/// warning). Precedence after seeding: CLI flag > env var > config file > default.
fn seed_env_from_config_file(path: &Path) -> Option<FileConfig> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return None, // missing file is silently ignored
    };
    let fc: FileConfig = match toml::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "lcp: warning: ignoring malformed config file {}: {e}",
                path.display()
            );
            return None;
        }
    };

    macro_rules! seed {
        ($var:literal, $val:expr) => {
            if let Some(v) = $val {
                if std::env::var($var).is_err() {
                    // SAFETY: called in fn main() before the tokio multi-thread
                    // runtime is constructed, so no other threads exist yet.
                    unsafe { std::env::set_var($var, v.to_string()) };
                }
            }
        };
    }

    seed!("LCP_PORT", fc.port);
    seed!("LCP_HOST", fc.host.as_deref());
    seed!("LCP_DB", fc.db.as_deref());
    seed!("LCP_TTL", fc.ttl);
    seed!("LCP_TIMEOUT", fc.timeout);
    seed!("LCP_ANTHROPIC_UPSTREAM", fc.anthropic_upstream.as_deref());
    seed!("LCP_OPENAI_UPSTREAM", fc.openai_upstream.as_deref());
    seed!("LCP_OPENROUTER_UPSTREAM", fc.openrouter_upstream.as_deref());
    seed!("LCP_GEMINI_UPSTREAM", fc.gemini_upstream.as_deref());

    Some(fc)
}

/// Expand a leading `~` to the user's home directory.
/// `~/foo` → `/home/user/foo`; `~` alone → `/home/user`; no `~` → unchanged.
fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}

/// Print the effective configuration as TOML. Options with built-in defaults
/// are always shown; optional options are commented out when unset.
fn print_config(cli: &Cli, ext: Option<&ExtensionsConfig>) {
    let db = cli
        .db
        .as_deref()
        .map(|p| expand_tilde(&p.to_string_lossy()))
        .unwrap_or_else(default_db_path)
        .display()
        .to_string();

    println!("port = {}", cli.port);
    println!("host = \"{}\"", cli.host);
    println!("db = \"{}\"", db);
    println!("ttl = {}", cli.ttl);
    println!("timeout = {}", cli.timeout);

    match &cli.anthropic_upstream {
        Some(u) => println!("anthropic_upstream = \"{}\"", u),
        None => println!("# anthropic_upstream = \"\""),
    }
    match &cli.openai_upstream {
        Some(u) => println!("openai_upstream = \"{}\"", u),
        None => println!("# openai_upstream = \"\""),
    }
    match &cli.openrouter_upstream {
        Some(u) => println!("openrouter_upstream = \"{}\"", u),
        None => println!("# openrouter_upstream = \"\""),
    }
    match &cli.gemini_upstream {
        Some(u) => println!("gemini_upstream = \"{}\"", u),
        None => println!("# gemini_upstream = \"\""),
    }

    println!();
    match ext
        .and_then(|e| e.doppel.as_ref())
        .and_then(|s| s.secrets_file.as_deref())
    {
        Some(p) => {
            println!("[extensions.doppel]");
            println!("secrets_file = \"{}\"", p);
        }
        None => {
            println!("# [extensions.doppel]");
            println!("# secrets_file = \"\"  # run: doppel init <path>");
        }
    }
}

fn default_db_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lcp")
        .join("cache.db")
}

/// Build the extension pipeline from the `[extensions]` section of the config.
///
/// - `[extensions.doppel]` absent → empty pipeline (no swapping).
/// - `[extensions.doppel]` present, no `secrets_file` → warning with setup instructions.
/// - `secrets_file` set, file missing or invalid → warning, no swapping.
/// - `secrets_file` set, file valid → `DoppelExt` registered.
fn build_extension_pipeline(ext: Option<&ExtensionsConfig>) -> ExtensionPipeline {
    let Some(ext) = ext else {
        return ExtensionPipeline::new();
    };

    let Some(doppel_cfg) = &ext.doppel else {
        return ExtensionPipeline::new();
    };

    let Some(raw_path) = &doppel_cfg.secrets_file else {
        tracing::warn!(
            "[extensions.doppel] is configured but `secrets_file` is not set; \
             doppel extension is disabled. \
             Add secrets_file = ~/.config/lcp/secrets.toml to [extensions.doppel], \
             then run `doppel init --patterns <path>` to create the file."
        );
        return ExtensionPipeline::new();
    };

    let path = expand_tilde(raw_path);

    match DoppelExt::from_secrets_file(&path) {
        Ok(swap) => {
            tracing::info!(path = %path.display(), "doppel extension loaded");
            ExtensionPipeline::new().register(swap)
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                "doppel secrets file could not be loaded; doppel extension is disabled. \
                 Run `doppel init {}` to create it, then restart lcp. Error: {e}",
                path.display(),
            );
            ExtensionPipeline::new()
        }
    }
}

fn main() -> Result<()> {
    // Seed env vars from config file before the tokio runtime starts.
    // This keeps set_var single-threaded and gives the correct precedence:
    //   CLI flag > env var > config file > built-in default.
    let config_path = config_path_from_args().unwrap_or_else(default_config_path);
    let file_config = seed_env_from_config_file(&config_path);

    let cli = Cli::parse();

    if cli.print_config {
        print_config(
            &cli,
            file_config.as_ref().and_then(|fc| fc.extensions.as_ref()),
        );
        return Ok(());
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli, file_config))
}

async fn run(cli: Cli, file_config: Option<FileConfig>) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("lcp=info".parse()?))
        .init();

    let db_path = cli
        .db
        .as_deref()
        .map(|p| expand_tilde(&p.to_string_lossy()))
        .unwrap_or_else(default_db_path);

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cache = Cache::open(&db_path, cli.ttl)?;
    tracing::info!(path = %db_path.display(), "cache database opened");

    let extensions =
        build_extension_pipeline(file_config.as_ref().and_then(|fc| fc.extensions.as_ref()));

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;

    let config = ServerConfig {
        addr,
        cache,
        timeout_seconds: cli.timeout,
        anthropic_upstream: cli.anthropic_upstream,
        openai_upstream: cli.openai_upstream,
        openrouter_upstream: cli.openrouter_upstream,
        gemini_upstream: cli.gemini_upstream,
        stream_channel_capacity: 32,
        extensions,
    };

    tracing::info!("set ANTHROPIC_BASE_URL=http://{addr}/anthropic");
    tracing::info!("set OPENAI_BASE_URL=http://{addr}/openai");
    tracing::info!("set OPENROUTER_BASE_URL=http://{addr}/openrouter");
    tracing::info!("set GEMINI_BASE_URL=http://{addr}/gemini");

    serve(config).await
}
