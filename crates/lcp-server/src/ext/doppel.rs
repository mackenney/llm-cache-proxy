//! DoppelExt — Phase 2/3 secret-swapping extension backed by `doppel`.
//!
//! [`Pattern`]s — built-in structural (Anthropic, OpenAI, etc.) or user-registered secrets — are applied
//! to the request body in Phase 2 before it reaches the upstream.  Detected
//! secrets are replaced with structurally-equivalent fakes; the originals are
//! restored in Phase 3 via `SseRestoreStream` before the response is written to
//! cache and returned to the client.
//!
//! # Phase interactions (per lcp-server SPEC §Pipeline and Cache Interaction)
//!
//! - **Phase 1 (not used):** identity — cache key includes the original body, so
//!   each unique secret combination gets its own cache entry.
//! - **Phase 2:** `doppel::swap` replaces secrets with fakes.  The
//!   `Entry` set and session key are placed in `SensitiveState` for Phase 3.
//! - **Phase 3:** For SSE responses, `SseRestoreStream` performs semantic-level
//!   unswapping (accumulate text across events, restore, redistribute). For
//!   non-SSE responses, `doppel::restore_stream` performs raw-byte
//!   Aho-Corasick restoration. The cache stores restored content; the wire
//!   carried only fakes.
//!
//! # SensitiveState layout (per request)
//!
//! | Key             | Value                                             |
//! |-----------------|---------------------------------------------------|
//! | `"entries"`     | `Entry::serialize_entries` JSON → UTF-8 string    |
//! | `"session_key"` | 32-byte session key encoded as 64 lowercase hex   |
//!
//! The session key is held in memory only, never written to disk, and never
//! appears in logs (`SensitiveState::Debug` is redacted).

use std::io;
use std::sync::Arc;

use bytes::Bytes;
use doppel::{Detector, Entry, Pattern, SessionKey};
use futures_util::future::BoxFuture;

use crate::extensions::{
    Extension, ProxyCtx, ResponseStream, SensitiveState, SensitiveStateBuilder,
};

use crate::ext::sse_restore::SseRestoreStream;

/// Error returned by [`DoppelExt::from_secrets_file`] when the secrets file
/// cannot be read or deserialized.
#[derive(Debug, thiserror::Error)]
pub enum DoppelExtLoadError {
    /// The secrets file could not be read from disk.
    #[error("cannot read patterns file: {0}")]
    Io(#[from] std::io::Error),
    /// The secrets file was read but could not be deserialized.
    #[error("invalid patterns file: {0}")]
    Patterns(#[from] doppel::SecretsFileError),
}

/// Extension that swaps detected secrets from request bodies before they are
/// forwarded to the upstream, and restores them in the response stream.
///
/// Construct with one or more [`Pattern`]s — structural built-ins from
/// [`doppel::patterns`] or registered secrets via [`doppel::register`] — and
/// register it with `ExtensionPipeline::register`.
///
/// # Example
///
/// ```ignore
/// use doppel::{register, patterns};
/// use lcp_server::{DoppelExt, ExtensionPipeline};
///
/// let pipeline = ExtensionPipeline::new().register(
///     DoppelExt::new(vec![
///         patterns::anthropic(),
///         patterns::openai_project(),
///         register(b"my-internal-token-long-enough").unwrap(),
///     ])
/// );
/// ```
pub struct DoppelExt {
    detector: Arc<Detector>,
}

impl DoppelExt {
    /// Load a `DoppelExt` from a patterns file on disk.
    ///
    /// Reads the TOML file, deserializes it via `SecretsFile::deserialize`, and
    /// calls `into_patterns()` to build the full pattern set.
    pub fn from_secrets_file(path: &std::path::Path) -> Result<Self, DoppelExtLoadError> {
        let bytes = std::fs::read(path)?;
        let pf = doppel::SecretsFile::deserialize(&bytes)?;
        let patterns = pf.to_patterns()?;
        Ok(Self::new(patterns))
    }

    /// Create a `DoppelExt` from an explicit list of patterns.
    pub fn new(patterns: Vec<Pattern>) -> Self {
        Self {
            detector: Arc::new(Detector::new(patterns)),
        }
    }
}

impl Extension for DoppelExt {
    fn name(&self) -> &'static str {
        "doppel"
    }

    /// Phase 2: swap the request body, store encrypted entries and the session
    /// key in `SensitiveStateBuilder` so Phase 3 can restore them.
    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        let detector = Arc::clone(&self.detector);
        Box::pin(async move {
            let result = detector.swap(&body)?;

            if result.entries.is_empty() {
                // No secrets detected — pass body through unchanged with empty state.
                return Ok((body, SensitiveStateBuilder::new()));
            }

            let entries_json = Entry::serialize_entries(&result.entries)?;
            let entries_str = String::from_utf8(entries_json)
                .map_err(|e| anyhow::anyhow!("entries JSON is not UTF-8: {e}"))?;

            let key_hex = hex::encode(result.session_key.as_bytes());

            let mut builder = SensitiveStateBuilder::new();
            builder.set("entries", &entries_str);
            builder.set("session_key", &key_hex);

            Ok((Bytes::from(result.payload), builder))
        })
    }

    /// Phase 3: wrap the response stream in `SseRestoreStream` to restore originals.
    fn on_response_stream(
        &self,
        ctx: ProxyCtx,
        state: SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        // Recover entries JSON and session key hex from SensitiveState.
        let Some(entries_json) = state.get("entries").map(|s| s.as_bytes().to_vec()) else {
            // No secrets were swapped — pass stream through unchanged.
            return stream;
        };
        let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
            return error_stream("SensitiveState has entries but no session_key".into());
        };
        drop(state);

        let entries = match Entry::deserialize_entries(&entries_json) {
            Ok(e) => e,
            Err(e) => return error_stream(format!("entries deserialization failed: {e}")),
        };

        let key_bytes: [u8; 32] = match hex::decode(&key_hex)
            .map_err(|e| e.to_string())
            .and_then(|v| v.try_into().map_err(|_| "expected 32 bytes".to_owned()))
        {
            Ok(b) => b,
            Err(e) => return error_stream(format!("session key decode failed: {e}")),
        };
        let session_key = SessionKey::from_bytes(key_bytes);

        Box::pin(SseRestoreStream::new(
            stream,
            entries,
            session_key,
            ctx.provider,
        ))
    }
}

/// Return a stream that immediately yields a single `io::Error`.
fn error_stream(msg: String) -> ResponseStream {
    Box::pin(futures_util::stream::once(async move {
        Err(io::Error::other(msg))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use doppel::patterns;
    use futures_util::StreamExt;

    // Synthetic test secrets matching built-in structural patterns.
    // These are NOT real credentials.
    const ANT: &[u8] = b"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const OPENAI: &[u8] = b"sk-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn ctx() -> ProxyCtx {
        ProxyCtx {
            provider: lcp_core::Provider::Anthropic,
            method: "POST".into(),
            path: "/v1/messages".into(),
            cache_key: Some("test".into()),
        }
    }

    #[tokio::test]
    async fn phase2_swaps_tier1_anthropic_key() {
        let ext = DoppelExt::new(vec![patterns::anthropic()]);
        let body = Bytes::from([b"key: ".as_slice(), ANT].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        assert!(
            !scrubbed.windows(ANT.len()).any(|w| w == ANT),
            "real key must not appear in swapped body"
        );
        assert!(
            state.get("entries").is_some(),
            "entries must be stored in SensitiveState"
        );
        assert!(
            state.get("session_key").is_some(),
            "session_key must be stored in SensitiveState"
        );
    }

    #[tokio::test]
    async fn phase2_no_secret_in_body_returns_unchanged() {
        let ext = DoppelExt::new(vec![patterns::anthropic()]);
        let body = Bytes::from(r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":5}"#);
        let (out, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        assert_eq!(
            out, body,
            "body without secrets must pass through unchanged"
        );
        assert!(
            state.get("entries").is_none(),
            "no entries when no secret detected"
        );
    }

    #[tokio::test]
    async fn phase3_restores_secret_in_response() {
        use futures_util::stream;

        let ext = DoppelExt::new(vec![patterns::anthropic()]);
        let body = Bytes::from([b"key: ".as_slice(), ANT].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        // Simulate a response that echoes the fake back.
        let echo = Bytes::from(scrubbed.to_vec());
        let raw_stream: ResponseStream =
            Box::pin(stream::once(async move { Ok::<Bytes, io::Error>(echo) }));

        let restored_stream = ext.on_response_stream(ctx(), state, raw_stream);
        let chunks: Vec<_> = restored_stream.collect().await;

        // restore_stream emits the prefix ("key: ") and the restored secret as
        // separate chunks; concatenate all to check the full restored output.
        let all_bytes: Vec<u8> = chunks
            .iter()
            .flat_map(|c| c.as_ref().unwrap().to_vec())
            .collect();
        assert!(
            all_bytes.windows(ANT.len()).any(|w| w == ANT),
            "Phase 3 must restore the real key in the response; got: {:?}",
            String::from_utf8_lossy(&all_bytes)
        );
    }

    #[tokio::test]
    async fn phase3_empty_state_passes_stream_unchanged() {
        use futures_util::stream;

        let ext = DoppelExt::new(vec![patterns::anthropic()]);
        // No secrets in body → empty SensitiveState.
        let body = Bytes::from(b"plain body".as_slice());
        let (_, builder) = ext.on_upstream_body(ctx(), body).await.unwrap();
        let state = builder.build();

        let payload = Bytes::from(b"response".as_slice());
        let raw_stream: ResponseStream = Box::pin(stream::once(async move {
            Ok::<Bytes, io::Error>(payload.clone())
        }));

        let out_stream = ext.on_response_stream(ctx(), state, raw_stream);
        let chunks: Vec<_> = out_stream.collect().await;
        let text = chunks[0].as_ref().unwrap();
        assert_eq!(text.as_ref(), b"response");
    }

    #[tokio::test]
    async fn phase2_swaps_openai_classic_key() {
        let ext = DoppelExt::new(vec![patterns::openai_classic()]);
        let body = Bytes::from([b"Authorization: Bearer ".as_slice(), OPENAI].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        assert!(
            !scrubbed.windows(OPENAI.len()).any(|w| w == OPENAI),
            "OpenAI key must be swapped"
        );
        assert!(state.get("entries").is_some());
    }
}
