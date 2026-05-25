use std::fmt;

/// Supported upstream LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Anthropic,
    OpenAi,
    OpenRouter,
    Gemini,
}

impl Provider {
    /// URL path prefix used to identify the provider in proxy requests.
    pub fn path_prefix(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::OpenRouter => "openrouter",
            Provider::Gemini => "gemini",
        }
    }

    /// Default upstream base URL for this provider.
    pub fn default_upstream(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com",
            Provider::OpenRouter => "https://openrouter.ai/api",
            Provider::Gemini => "https://generativelanguage.googleapis.com",
        }
    }

    /// Parse from a path prefix string.
    pub fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAi),
            "openrouter" => Some(Provider::OpenRouter),
            "gemini" => Some(Provider::Gemini),
            _ => None,
        }
    }

    /// Extract a model name from a URL path for providers that encode it there.
    ///
    /// Returns `None` for providers that specify the model in the request body
    /// instead of the URL (Anthropic, OpenAI, OpenRouter).
    pub fn extract_model_from_path(self, path: &str) -> Option<String> {
        match self {
            Provider::Gemini => extract_gemini_model(path),
            _ => None,
        }
    }

    /// Provider-specific request body fields to strip during cache key normalization.
    /// Does not include `stream`, which is stripped for all providers in `hash::normalize_body`.
    pub fn normalization_strip_fields(self) -> &'static [&'static str] {
        match self {
            Provider::Anthropic => &["metadata"],
            Provider::OpenAi => &["user"],
            Provider::OpenRouter => &["user", "provider", "route"],
            Provider::Gemini => &[],
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path_prefix())
    }
}

/// Parse the model name from a Gemini URL path.
///
/// Gemini encodes the model as `/models/{model}:{verb}`. This extracts the
/// segment between `/models/` and the first `:`. Returns `None` if the pattern
/// is absent or if the extracted model name is empty.
fn extract_gemini_model(path: &str) -> Option<String> {
    const MARKER: &str = "/models/";
    let idx = path.find(MARKER)?;
    let after_marker = &path[idx + MARKER.len()..];
    let end = after_marker.find(':')?;
    let model = &after_marker[..end];
    if model.is_empty() {
        return None;
    }
    Some(model.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_extract_model_from_generate_content() {
        assert_eq!(
            Provider::Gemini
                .extract_model_from_path("v1beta/models/gemini-2.5-flash:generateContent"),
            Some("gemini-2.5-flash".to_owned())
        );
    }

    #[test]
    fn gemini_extract_model_from_stream_generate_content() {
        assert_eq!(
            Provider::Gemini
                .extract_model_from_path("v1beta/models/gemini-pro:streamGenerateContent"),
            Some("gemini-pro".to_owned())
        );
    }

    #[test]
    fn gemini_extract_model_with_dashes_and_version() {
        assert_eq!(
            Provider::Gemini
                .extract_model_from_path("v1beta/models/gemini-2.0-flash-exp:countTokens"),
            Some("gemini-2.0-flash-exp".to_owned())
        );
    }

    #[test]
    fn gemini_extract_model_empty_returns_none() {
        assert_eq!(
            Provider::Gemini.extract_model_from_path("v1beta/models/:generateContent"),
            None
        );
    }

    #[test]
    fn gemini_extract_model_no_marker_returns_none() {
        assert_eq!(
            Provider::Gemini.extract_model_from_path("v1beta/something/else"),
            None
        );
    }

    #[test]
    fn gemini_extract_model_no_colon_returns_none() {
        assert_eq!(
            Provider::Gemini.extract_model_from_path("v1beta/models/no-colon"),
            None
        );
    }

    #[test]
    fn non_gemini_providers_return_none_from_path() {
        let path = "v1beta/models/gemini-2.5-flash:generateContent";
        assert_eq!(Provider::Anthropic.extract_model_from_path(path), None);
        assert_eq!(Provider::OpenAi.extract_model_from_path(path), None);
        assert_eq!(Provider::OpenRouter.extract_model_from_path(path), None);
    }
}
