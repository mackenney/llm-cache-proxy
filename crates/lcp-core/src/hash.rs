use crate::Provider;

/// Compute the cache key for a request.
///
/// The key is a BLAKE3 hex digest of the normalized request body. Normalization
/// strips fields that carry no semantic content (e.g. `stream`) and sorts JSON
/// keys so that logically identical requests with different field orderings
/// produce the same key.
///
/// The `method` and `path` are included so that structurally identical bodies
/// sent to different endpoints never collide.
pub fn cache_key(provider: Provider, method: &str, path: &str, body: &[u8]) -> String {
    let normalized = normalize_body(provider, body);
    let mut hasher = blake3::Hasher::new();
    hasher.update(method.as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(normalized.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Normalize a JSON request body for stable hashing.
///
/// - Parses as JSON; if parsing fails, returns the raw body as-is (still
///   produces a stable key for that exact byte sequence).
/// - Strips semantic-free fields: `stream`.
/// - Sorts all object keys recursively.
fn normalize_body(provider: Provider, body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return String::from_utf8_lossy(body).into_owned();
    };
    strip_fields(&mut value, &["stream"]);
    strip_fields(&mut value, provider.normalization_strip_fields());
    sort_keys(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
}

fn strip_fields(value: &mut serde_json::Value, fields: &[&str]) {
    if let serde_json::Value::Object(map) = value {
        for field in fields {
            map.remove(*field);
        }
        for v in map.values_mut() {
            strip_fields(v, fields);
        }
    }
}

fn sort_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: serde_json::Map<String, serde_json::Value> = {
                let mut pairs: Vec<(String, serde_json::Value)> = map
                    .iter_mut()
                    .map(|(k, v)| {
                        sort_keys(v);
                        (k.clone(), v.clone())
                    })
                    .collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                pairs.into_iter().collect()
            };
            *map = sorted;
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                sort_keys(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Provider;

    #[test]
    fn identical_bodies_same_key() {
        let body = br#"{"model":"claude-opus-4","messages":[{"role":"user","content":"hello"}]}"#;
        assert_eq!(
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", body),
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", body)
        );
    }

    #[test]
    fn stream_field_stripped() {
        let with_stream = br#"{"model":"claude-opus-4","stream":true,"messages":[]}"#;
        let without_stream = br#"{"model":"claude-opus-4","messages":[]}"#;
        assert_eq!(
            cache_key(
                Provider::Anthropic,
                "POST",
                "/anthropic/v1/messages",
                with_stream
            ),
            cache_key(
                Provider::Anthropic,
                "POST",
                "/anthropic/v1/messages",
                without_stream
            ),
        );
    }

    #[test]
    fn key_order_independent() {
        let a = br#"{"messages":[],"model":"claude-opus-4"}"#;
        let b = br#"{"model":"claude-opus-4","messages":[]}"#;
        assert_eq!(
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", a),
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", b),
        );
    }

    #[test]
    fn different_bodies_different_keys() {
        let a = br#"{"model":"claude-opus-4","messages":[{"role":"user","content":"hello"}]}"#;
        let b = br#"{"model":"claude-opus-4","messages":[{"role":"user","content":"world"}]}"#;
        assert_ne!(
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", a),
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", b),
        );
    }

    #[test]
    fn different_paths_different_keys() {
        let body = br#"{"model":"gpt-4o","messages":[]}"#;
        assert_ne!(
            cache_key(
                Provider::Anthropic,
                "POST",
                "/openai/v1/chat/completions",
                body
            ),
            cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", body),
        );
    }

    #[test]
    fn invalid_json_hashes_raw() {
        let body = b"not json at all";
        let k1 = cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", body);
        let k2 = cache_key(Provider::Anthropic, "POST", "/anthropic/v1/messages", body);
        assert_eq!(k1, k2);
    }
}
