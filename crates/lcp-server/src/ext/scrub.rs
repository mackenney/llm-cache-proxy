//! ScrubExt — Phase 2/3 secret-scrubbing extension backed by `its-classified`.
//!
//! Registered [`Pattern`]s (Tier 1 structural or Tier 2 registered) are applied
//! to the request body in Phase 2 before it reaches the upstream.  Detected
//! secrets are replaced with structurally-equivalent fakes; the originals are
//! restored in Phase 3 via `UnscrubStream` before the response is written to
//! cache and returned to the client.
//!
//! # Phase interactions (per lcp-server SPEC §Pipeline and Cache Interaction)
//!
//! - **Phase 1 (not used):** identity — cache key includes the original body, so
//!   each unique secret combination gets its own cache entry.
//! - **Phase 2:** `its_classified::scrub` replaces secrets with fakes.  The
//!   `Entry` set and session key are placed in `SensitiveState` for Phase 3.
//! - **Phase 3:** `its_classified::unscrub_stream` restores originals in every
//!   response chunk.  The cache stores the restored content; the wire carried
//!   only fakes.
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

use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use its_classified::types::{Entry, Pattern, SessionKey};
use its_classified::{scrub, unscrub_stream};

use crate::extensions::{
    Extension, ProxyCtx, ResponseStream, SensitiveState, SensitiveStateBuilder,
};

/// Extension that scrubs detected secrets from request bodies before they are
/// forwarded to the upstream, and restores them in the response stream.
///
/// Construct with one or more [`Pattern`]s (Tier 1 built-ins from
/// [`its_classified::tier1::patterns`] or Tier 2 via [`its_classified::register`]).
///
/// # Example
///
/// ```ignore
/// use its_classified::{register, tier1::patterns};
/// use lcp_server::ScrubExt;
///
/// let pipeline = ExtensionPipeline::new().register(
///     ScrubExt::new(vec![
///         patterns::anthropic(),
///         patterns::openai_project(),
///         register(b"my-internal-token-long-enough").unwrap(),
///     ])
/// );
/// ```
/// Error returned by [`ScrubExt::from_patterns_file`].
#[derive(Debug, thiserror::Error)]
pub enum ScrubExtLoadError {
    #[error("cannot read patterns file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid patterns file: {0}")]
    Patterns(#[from] its_classified::PatternsFileError),
}

pub struct ScrubExt {
    patterns: Vec<Pattern>,
}

impl ScrubExt {
    /// Load a `ScrubExt` from a patterns file on disk.
    ///
    /// Reads the TOML file, deserializes it via `PatternsFile::deserialize`, and
    /// calls `into_patterns()` to build the full pattern set.
    pub fn from_patterns_file(path: &std::path::Path) -> Result<Self, ScrubExtLoadError> {
        let bytes = std::fs::read(path)?;
        let pf = its_classified::PatternsFile::deserialize(&bytes)?;
        let patterns = pf.into_patterns()?;
        Ok(Self::new(patterns))
    }

    /// Create a `ScrubExt` from an explicit list of patterns.
    pub fn new(patterns: Vec<Pattern>) -> Self {
        Self { patterns }
    }
}

impl Extension for ScrubExt {
    fn name(&self) -> &'static str {
        "scrub"
    }

    /// Phase 2: scrub the request body, store encrypted entries and the session
    /// key in `SensitiveStateBuilder` so Phase 3 can restore them.
    fn on_upstream_body(
        &self,
        _ctx: ProxyCtx,
        body: Bytes,
    ) -> BoxFuture<'static, Result<(Bytes, SensitiveStateBuilder), anyhow::Error>> {
        let patterns = self.patterns.clone();
        Box::pin(async move {
            let result = scrub(&body, &patterns)?;

            if result.entries.is_empty() {
                // No secrets detected — pass body through unchanged with empty state.
                return Ok((body, SensitiveStateBuilder::new()));
            }

            let entries_json = Entry::serialize_entries(&result.entries)?;
            let entries_str = String::from_utf8(entries_json)
                .map_err(|e| anyhow::anyhow!("entries JSON is not UTF-8: {e}"))?;

            let key_hex = encode_key_hex(result.session_key.as_bytes());

            let mut builder = SensitiveStateBuilder::new();
            builder.set("entries", &entries_str);
            builder.set("session_key", &key_hex);

            Ok((Bytes::from(result.payload), builder))
        })
    }

    /// Phase 3: wrap the response stream in `UnscrubStream` to restore originals.
    fn on_response_stream(
        &self,
        _ctx: ProxyCtx,
        state: SensitiveState,
        stream: ResponseStream,
    ) -> ResponseStream {
        // Recover entries JSON and session key hex from SensitiveState.
        let Some(entries_json) = state.get("entries").map(|s| s.as_bytes().to_vec()) else {
            // No secrets were scrubbed — pass stream through unchanged.
            return stream;
        };
        let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
            return stream;
        };
        drop(state);

        let entries = match Entry::deserialize_entries(&entries_json) {
            Ok(e) => e,
            Err(e) => return error_stream(format!("entries deserialization failed: {e}")),
        };

        let key_bytes = match decode_key_hex(&key_hex) {
            Ok(b) => b,
            Err(e) => return error_stream(format!("session key decode failed: {e}")),
        };
        let session_key = SessionKey::from_bytes(key_bytes);

        match unscrub_stream(stream, entries, session_key) {
            Ok(us) => Box::pin(us.map(|r| r.map_err(|e| io::Error::other(e.to_string())))),
            Err(e) => error_stream(format!("unscrub_stream construction failed: {e}")),
        }
    }
}

/// Encode 32 key bytes as 64 lowercase hex characters.
fn encode_key_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Decode 64 lowercase hex characters back into 32 bytes.
fn decode_key_hex(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 {
        return Err(format!("expected 64 hex chars, got {}", hex.len()));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("invalid hex byte: {b:#x}")),
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
    use its_classified::tier1::patterns;

    // Synthetic test secrets matching Tier 1 structural patterns.
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
    async fn phase2_scrubs_tier1_anthropic_key() {
        let ext = ScrubExt::new(vec![patterns::anthropic()]);
        let body = Bytes::from([b"key: ".as_slice(), ANT].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        assert!(
            !scrubbed.windows(ANT.len()).any(|w| w == ANT),
            "real key must not appear in scrubbed body"
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
        let ext = ScrubExt::new(vec![patterns::anthropic()]);
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

        let ext = ScrubExt::new(vec![patterns::anthropic()]);
        let body = Bytes::from([b"key: ".as_slice(), ANT].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        // Simulate a response that echoes the fake back.
        let echo = Bytes::from(scrubbed.to_vec());
        let raw_stream: ResponseStream =
            Box::pin(stream::once(async move { Ok::<Bytes, io::Error>(echo) }));

        let restored_stream = ext.on_response_stream(ctx(), state, raw_stream);
        let chunks: Vec<_> = restored_stream.collect().await;

        let text = String::from_utf8(chunks[0].as_ref().unwrap().to_vec()).unwrap();
        assert!(
            text.as_bytes().windows(ANT.len()).any(|w| w == ANT),
            "Phase 3 must restore the real key in the response; got: {text:?}"
        );
    }

    #[tokio::test]
    async fn phase3_empty_state_passes_stream_unchanged() {
        use futures_util::stream;

        let ext = ScrubExt::new(vec![patterns::anthropic()]);
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

    #[test]
    fn key_hex_round_trip() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        let hex = encode_key_hex(&bytes);
        assert_eq!(hex.len(), 64);
        let decoded = decode_key_hex(&hex).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[tokio::test]
    async fn phase2_scrubs_openai_classic_key() {
        let ext = ScrubExt::new(vec![patterns::openai_classic()]);
        let body = Bytes::from([b"Authorization: Bearer ".as_slice(), OPENAI].concat());
        let (scrubbed, builder) = ext.on_upstream_body(ctx(), body.clone()).await.unwrap();
        let state = builder.build();

        assert!(
            !scrubbed.windows(OPENAI.len()).any(|w| w == OPENAI),
            "OpenAI key must be scrubbed"
        );
        assert!(state.get("entries").is_some());
    }
}
