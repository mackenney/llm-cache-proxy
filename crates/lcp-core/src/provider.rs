use std::fmt;

/// Supported upstream LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Anthropic,
    OpenAi,
    OpenRouter,
}

impl Provider {
    /// URL path prefix used to identify the provider in proxy requests.
    pub fn path_prefix(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::OpenRouter => "openrouter",
        }
    }

    /// Default upstream base URL for this provider.
    pub fn default_upstream(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com",
            Provider::OpenRouter => "https://openrouter.ai/api",
        }
    }

    /// Parse from a path prefix string.
    pub fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAi),
            "openrouter" => Some(Provider::OpenRouter),
            _ => None,
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path_prefix())
    }
}
