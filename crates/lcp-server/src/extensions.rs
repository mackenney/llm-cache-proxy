use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::Stream;
use futures_util::future::BoxFuture;
use lcp_core::Provider;

/// Pinned, boxed response byte stream passed through Phase 3 hooks.
pub type ResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// Construction surface for [`SensitiveState`]. Produced and filled in Phase 2,
/// sealed via [`build`](SensitiveStateBuilder::build) before being handed to the
/// framework.
#[derive(Default)]
pub struct SensitiveStateBuilder {
    inner: HashMap<String, String>,
}

impl SensitiveStateBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.inner.insert(key.into(), value.into());
        self
    }

    pub fn build(self) -> SensitiveState {
        SensitiveState {
            inner: Arc::new(self.inner),
        }
    }
}

/// Immutable per-request, per-extension state store.
///
/// Carries extension state across the Phase 2 → Phase 3 boundary.
///
/// # Framework guarantees
///
/// - Contents are never logged, traced, or printed (`Debug` renders as
///   `SensitiveState { <redacted> }`).
/// - Never shared across extensions; each extension receives only the state
///   it produced.
/// - Never persisted to any storage medium.
/// - Dropped when the request's response stream is fully consumed or errors out.
#[derive(Clone)]
pub struct SensitiveState {
    inner: Arc<HashMap<String, String>>,
}

impl SensitiveState {
    /// Returns the value for `key`, or `None` if absent.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key).map(|s| s.as_str())
    }
}

/// Debug MUST NOT reveal contents — part of the non-inspection guarantee.
impl fmt::Debug for SensitiveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SensitiveState {{ <redacted> }}")
    }
}

/// Read-only snapshot of the in-flight request passed to extension hooks.
///
/// `cache_key` is `None` in Phase 1 (not yet computed) and `Some` in
/// Phase 2 and Phase 3.
#[derive(Clone, Debug)]
pub struct ProxyCtx {
    pub provider: Provider,
    pub method: String,
    pub path: String,
    pub cache_key: Option<String>,
}

/// An extension registered in the [`ExtensionPipeline`].
///
/// All three hook methods have default identity implementations. An extension
/// that only needs one hook may leave the others as defaults.
///
/// Implementors must be `Send + Sync + 'static`.
pub trait Extension: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// Phase 1 — fires before cache key computation on every proxied request.
    ///
    /// Default: return `body` unchanged.
    fn on_request_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<Bytes, anyhow::Error>> {
        Box::pin(async move { Ok(body) })
    }

    /// Phase 2 — fires on cache miss, after cache key computation, before the
    /// upstream request is sent.
    ///
    /// Returns the body to place on the wire and a [`SensitiveStateBuilder`]
    /// that the framework seals and passes exclusively to this extension's
    /// Phase 3 hook for this request.
    ///
    /// Default: return `body` unchanged with an empty state.
    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        Box::pin(async move { Ok((body, SensitiveStateBuilder::new())) })
    }

    /// Phase 3 — fires on cache miss, after the upstream responds. Wraps the
    /// response stream.
    ///
    /// `state` is the [`SensitiveState`] sealed from the
    /// [`SensitiveStateBuilder`] produced by this same extension's Phase 2
    /// hook for this same request.
    ///
    /// Default: return `stream` unchanged.
    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        _state: SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        stream
    }
}

/// Ordered collection of [`Extension`]s run on each proxied request.
///
/// An empty pipeline is a no-op. Extensions are called in registration order.
/// `Debug` shows only the extension count — never extension internals.
impl fmt::Debug for ExtensionPipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionPipeline")
            .field("extensions", &self.extensions.len())
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct ExtensionPipeline {
    extensions: Vec<Arc<dyn Extension>>,
}
impl ExtensionPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extension. Extensions run in registration order.
    pub fn register(mut self, ext: impl Extension) -> Self {
        self.extensions.push(Arc::new(ext));
        self
    }

    /// Phase 1: run all `on_request_body` hooks before cache key computation.
    ///
    /// Runs on every proxied request including bypass and cache-hit requests.
    /// On error, fails closed — the caller must not proceed with the request.
    pub async fn run_phase1(&self, ctx: ProxyCtx, mut body: Bytes) -> Result<Bytes, anyhow::Error> {
        for ext in &self.extensions {
            body = ext.on_request_body(ctx.clone(), body).await?;
        }
        Ok(body)
    }

    /// Phase 2: run all `on_upstream_body` hooks after cache key computation.
    ///
    /// Only called on cache misses. Returns the wire body and one
    /// [`SensitiveState`] per registered extension, indexed by registration
    /// order. On error, fails closed — the upstream request must not be sent.
    pub async fn run_phase2(
        &self,
        ctx: ProxyCtx,
        mut body: Bytes,
    ) -> Result<(Bytes, Vec<SensitiveState>), anyhow::Error> {
        let mut states = Vec::with_capacity(self.extensions.len());
        for ext in &self.extensions {
            let (new_body, builder) = ext.on_upstream_body(ctx.clone(), body).await?;
            body = new_body;
            states.push(builder.build());
        }
        Ok((body, states))
    }

    /// Phase 3: wrap the response stream through all `on_response_stream` hooks.
    ///
    /// Only called on cache misses. Each extension receives exclusively its own
    /// [`SensitiveState`] from the paired Phase 2 call — never another
    /// extension's state. `states` must be the `Vec` returned by
    /// [`run_phase2`](Self::run_phase2) for this request.
    pub fn run_phase3(
        &self,
        ctx: ProxyCtx,
        states: Vec<SensitiveState>,
        mut stream: ResponseStream,
    ) -> ResponseStream {
        debug_assert_eq!(
            self.extensions.len(),
            states.len(),
            "run_phase3: states/extensions length mismatch (got {} states for {} extensions)",
            states.len(),
            self.extensions.len(),
        );
        for (ext, state) in self.extensions.iter().zip(states) {
            stream = ext.on_response_stream(ctx.clone(), state, stream);
        }
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_state_debug_is_redacted() {
        let mut b = SensitiveStateBuilder::new();
        b.set("secret_key", "super_secret_value");
        let state = b.build();
        let debug_output = format!("{:?}", state);
        assert!(!debug_output.contains("super_secret_value"));
        assert!(!debug_output.contains("secret_key"));
        assert!(debug_output.contains("redacted"));
    }

    #[test]
    fn sensitive_state_get_returns_value() {
        let mut b = SensitiveStateBuilder::new();
        b.set("k", "v");
        let state = b.build();
        assert_eq!(state.get("k"), Some("v"));
        assert_eq!(state.get("missing"), None);
    }

    #[test]
    fn empty_pipeline_phase1_is_identity() {
        let pipeline = ExtensionPipeline::new();
        let body = Bytes::from("hello");
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run_phase1(
                ProxyCtx {
                    provider: lcp_core::Provider::Anthropic,
                    method: "POST".into(),
                    path: "/v1/messages".into(),
                    cache_key: None,
                },
                body.clone(),
            ))
            .unwrap();
        assert_eq!(result, body);
    }

    #[test]
    fn empty_pipeline_phase2_returns_empty_states() {
        let pipeline = ExtensionPipeline::new();
        let body = Bytes::from("hello");
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(pipeline.run_phase2(
                ProxyCtx {
                    provider: lcp_core::Provider::Anthropic,
                    method: "POST".into(),
                    path: "/v1/messages".into(),
                    cache_key: Some("abc123".into()),
                },
                body.clone(),
            ))
            .unwrap();
        assert_eq!(result.0, body);
        assert!(result.1.is_empty());
    }
}
