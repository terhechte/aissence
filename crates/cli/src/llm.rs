//! Configuration for local LLM summarization.
//!
//! Values are layered in this order: built-in OpenRouter defaults, an optional
//! TOML file, environment variables, then explicit CLI overrides.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Error;

pub const DEFAULT_CONFIG_FILE: &str = ".aissence.toml";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_MODEL: &str = "inclusionai/ling-2.6-flash";

pub const ENV_API: &str = "AISSENCE_LLM_API";
pub const ENV_BASE_URL: &str = "AISSENCE_LLM_BASE_URL";
pub const ENV_API_KEY: &str = "AISSENCE_LLM_API_KEY";
pub const ENV_API_KEY_ENV: &str = "AISSENCE_LLM_API_KEY_ENV";
pub const ENV_MODEL: &str = "AISSENCE_LLM_MODEL";
pub const ENV_PROMPT_MODEL: &str = "AISSENCE_LLM_PROMPT_MODEL";
pub const ENV_FALLBACK_MODEL: &str = "AISSENCE_LLM_FALLBACK_MODEL";
pub const ENV_PROVIDER: &str = "AISSENCE_LLM_PROVIDER";

pub const ENV_OPENROUTER_API_KEY: &str = "OPENROUTER_API_KEY";
pub const ENV_OPENROUTER_MODEL: &str = "OPENROUTER_MODEL";
pub const ENV_OPENROUTER_PROMPT_MODEL: &str = "OPENROUTER_PROMPT_MODEL";
pub const ENV_OPENROUTER_FALLBACK_MODEL: &str = "OPENROUTER_FALLBACK_MODEL";
pub const ENV_OPENROUTER_PROVIDER: &str = "OPENROUTER_PROVIDER";

/// The request dialect used by the configured endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LlmApi {
    #[default]
    OpenRouter,
    OpenaiCompatible,
}

impl LlmApi {
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Ok(Self::OpenRouter),
            "openai-compatible" | "openai_compatible" | "openai" => Ok(Self::OpenaiCompatible),
            other => Err(Error::LlmConfig(format!(
                "unknown API kind {other:?}; expected openrouter or openai-compatible"
            ))),
        }
    }
}

/// Fully resolved configuration used to construct a [`crate::Summarizer`].
#[derive(Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub api: LlmApi,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub prompt_model: Option<String>,
    pub fallback_model: Option<String>,
    pub provider: Option<String>,
}

/// Highest-precedence values supplied by a caller, normally the CLI.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LlmConfigOverrides {
    pub api: Option<LlmApi>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub model: Option<String>,
    pub prompt_model: Option<String>,
    pub fallback_model: Option<String>,
    pub provider: Option<String>,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LlmConfig")
            .field("api", &self.api)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model", &self.model)
            .field("prompt_model", &self.prompt_model)
            .field("fallback_model", &self.fallback_model)
            .field("provider", &self.provider)
            .finish()
    }
}

impl std::fmt::Debug for LlmConfigOverrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LlmConfigOverrides")
            .field("api", &self.api)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("api_key_env", &self.api_key_env)
            .field("model", &self.model)
            .field("prompt_model", &self.prompt_model)
            .field("fallback_model", &self.fallback_model)
            .field("provider", &self.provider)
            .finish()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    #[serde(default)]
    llm: PartialConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialConfig {
    api: Option<LlmApi>,
    base_url: Option<String>,
    api_key: Option<String>,
    api_key_env: Option<String>,
    model: Option<String>,
    prompt_model: Option<String>,
    fallback_model: Option<String>,
    provider: Option<String>,
}

impl LlmConfig {
    /// Load project configuration and apply environment and caller overrides.
    ///
    /// If `config_path` is omitted, `<project>/.aissence.toml` is used when it
    /// exists. An explicitly named file must exist and parse successfully.
    pub fn load(
        project: &Path,
        config_path: Option<&Path>,
        overrides: LlmConfigOverrides,
    ) -> Result<Self, Error> {
        let discovered = project.join(DEFAULT_CONFIG_FILE);
        let path = config_path
            .map(Path::to_path_buf)
            .or_else(|| discovered.is_file().then_some(discovered));
        let file = path
            .as_deref()
            .map(read_config)
            .transpose()?
            .unwrap_or_default();
        let effective_api = overrides
            .api
            .or(nonempty_env(ENV_API)
                .map(|value| LlmApi::parse(&value))
                .transpose()?)
            .or(file.api)
            .unwrap_or_default();
        Self::resolve(file, environment_config(effective_api)?, overrides)
    }

    /// Resolve configuration from environment variables only.
    pub fn from_environment() -> Result<Self, Error> {
        let effective_api = nonempty_env(ENV_API)
            .map(|value| LlmApi::parse(&value))
            .transpose()?
            .unwrap_or_default();
        Self::resolve(
            PartialConfig::default(),
            environment_config(effective_api)?,
            LlmConfigOverrides::default(),
        )
    }

    /// Resolve only the legacy OpenRouter environment contract.
    ///
    /// The hosted server uses this deliberately so client-side generic LLM
    /// settings cannot weaken its OpenRouter routing and privacy policy.
    pub fn openrouter_from_environment() -> Result<Self, Error> {
        let api_key = nonempty_env(ENV_OPENROUTER_API_KEY)
            .ok_or(Error::MissingEnv(ENV_OPENROUTER_API_KEY))?;
        let model = nonempty_env(ENV_OPENROUTER_MODEL).unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let fallback_model =
            nonempty_env(ENV_OPENROUTER_FALLBACK_MODEL).filter(|fallback| fallback != &model);
        let prompt_model =
            nonempty_env(ENV_OPENROUTER_PROMPT_MODEL).filter(|value| value != &model);
        Ok(Self {
            api: LlmApi::OpenRouter,
            base_url: OPENROUTER_BASE_URL.to_string(),
            api_key: Some(api_key),
            model,
            prompt_model,
            fallback_model,
            provider: nonempty_env(ENV_OPENROUTER_PROVIDER),
        })
    }

    fn resolve(
        file: PartialConfig,
        environment: PartialConfig,
        overrides: LlmConfigOverrides,
    ) -> Result<Self, Error> {
        let mut partial = PartialConfig {
            api: Some(LlmApi::OpenRouter),
            ..PartialConfig::default()
        };
        partial.apply(file);
        partial.apply(environment);
        partial.apply(PartialConfig {
            api: overrides.api,
            base_url: overrides.base_url,
            api_key: overrides.api_key,
            api_key_env: overrides.api_key_env,
            model: overrides.model,
            prompt_model: overrides.prompt_model,
            fallback_model: overrides.fallback_model,
            provider: overrides.provider,
        });

        let api = partial.api.unwrap_or_default();
        let base_url = match (partial.base_url, api) {
            (Some(value), _) => nonempty_value(Some(value), "base_url")?,
            (None, LlmApi::OpenRouter) => OPENROUTER_BASE_URL.to_string(),
            (None, LlmApi::OpenaiCompatible) => {
                return Err(Error::LlmConfig(
                    "base_url is required for an OpenAI-compatible API".to_string(),
                ));
            }
        };
        validate_base_url(&base_url)?;
        let model = match (partial.model, api) {
            (Some(value), _) => nonempty_value(Some(value), "model")?,
            (None, LlmApi::OpenRouter) => DEFAULT_MODEL.to_string(),
            (None, LlmApi::OpenaiCompatible) => {
                return Err(Error::LlmConfig(
                    "model is required for an OpenAI-compatible API".to_string(),
                ));
            }
        };
        let provider = partial.provider.and_then(nonempty);
        let prompt_model = partial
            .prompt_model
            .and_then(nonempty)
            .filter(|value| value != &model);
        let fallback_model = partial
            .fallback_model
            .and_then(nonempty)
            .filter(|fallback| fallback != &model);
        if api == LlmApi::OpenaiCompatible && provider.is_some() {
            return Err(Error::LlmConfig(
                "provider is an OpenRouter-only option".to_string(),
            ));
        }

        let api_key =
            match partial.api_key_env.and_then(nonempty) {
                Some(name) => Some(std::env::var(&name).ok().and_then(nonempty).ok_or_else(
                    || {
                        Error::LlmConfig(format!(
                            "API key environment variable {name:?} is not set or is empty"
                        ))
                    },
                )?),
                None => partial.api_key.and_then(nonempty),
            };
        if api == LlmApi::OpenRouter && api_key.is_none() {
            return Err(Error::MissingEnv(ENV_OPENROUTER_API_KEY));
        }

        Ok(Self {
            api,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            prompt_model,
            fallback_model,
            provider,
        })
    }
}

impl PartialConfig {
    fn apply(&mut self, higher: Self) {
        if higher.api.is_some() {
            self.api = higher.api;
        }
        if higher.base_url.is_some() {
            self.base_url = higher.base_url;
        }
        if higher.model.is_some() {
            self.model = higher.model;
        }
        if higher.prompt_model.is_some() {
            self.prompt_model = higher.prompt_model;
        }
        if higher.fallback_model.is_some() {
            self.fallback_model = higher.fallback_model;
        }
        if higher.provider.is_some() {
            self.provider = higher.provider;
        }
        // A key reference and a literal key are mutually exclusive sources.
        if higher.api_key_env.is_some() {
            self.api_key_env = higher.api_key_env;
            self.api_key = None;
        } else if higher.api_key.is_some() {
            self.api_key = higher.api_key;
            self.api_key_env = None;
        }
    }
}

fn read_config(path: &Path) -> Result<PartialConfig, Error> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| Error::LlmConfig(format!("cannot read {}: {error}", path.display())))?;
    let document: ConfigDocument = toml::from_str(&contents)
        .map_err(|error| Error::LlmConfig(format!("cannot parse {}: {error}", path.display())))?;
    Ok(document.llm)
}

fn environment_config(base_api: LlmApi) -> Result<PartialConfig, Error> {
    let api = nonempty_env(ENV_API)
        .map(|value| LlmApi::parse(&value))
        .transpose()?;
    // Preserve the legacy OpenRouter environment when no generic API kind was
    // selected. Generic variables always take precedence.
    let use_legacy = base_api == LlmApi::OpenRouter;
    Ok(PartialConfig {
        api,
        base_url: nonempty_env(ENV_BASE_URL),
        api_key: nonempty_env(ENV_API_KEY).or_else(|| {
            use_legacy
                .then(|| nonempty_env(ENV_OPENROUTER_API_KEY))
                .flatten()
        }),
        api_key_env: nonempty_env(ENV_API_KEY_ENV),
        model: nonempty_env(ENV_MODEL).or_else(|| {
            use_legacy
                .then(|| nonempty_env(ENV_OPENROUTER_MODEL))
                .flatten()
        }),
        prompt_model: nonempty_env(ENV_PROMPT_MODEL).or_else(|| {
            use_legacy
                .then(|| nonempty_env(ENV_OPENROUTER_PROMPT_MODEL))
                .flatten()
        }),
        fallback_model: nonempty_env(ENV_FALLBACK_MODEL).or_else(|| {
            use_legacy
                .then(|| nonempty_env(ENV_OPENROUTER_FALLBACK_MODEL))
                .flatten()
        }),
        provider: nonempty_env(ENV_PROVIDER).or_else(|| {
            use_legacy
                .then(|| nonempty_env(ENV_OPENROUTER_PROVIDER))
                .flatten()
        }),
    })
}

fn validate_base_url(value: &str) -> Result<(), Error> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|error| Error::LlmConfig(format!("invalid base_url {value:?}: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(Error::LlmConfig(
            "base_url must be an absolute http or https URL".to_string(),
        ));
    }
    Ok(())
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(nonempty)
}

fn nonempty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn nonempty_value(value: Option<String>, name: &str) -> Result<String, Error> {
    value
        .and_then(nonempty)
        .ok_or_else(|| Error::LlmConfig(format!("{name} cannot be empty")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_compatible_file() {
        let document: ConfigDocument = toml::from_str(
            r#"
                [llm]
                api = "openai-compatible"
                base_url = "http://127.0.0.1:11434/v1/"
                model = "qwen2.5-coder:7b"
                prompt_model = "qwen2.5-coder:3b"
                fallback_model = "qwen2.5-coder:32b"
            "#,
        )
        .unwrap();
        let config = LlmConfig::resolve(
            document.llm,
            PartialConfig::default(),
            LlmConfigOverrides::default(),
        )
        .unwrap();

        assert_eq!(config.api, LlmApi::OpenaiCompatible);
        assert_eq!(config.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(config.api_key, None);
        assert_eq!(config.model, "qwen2.5-coder:7b");
        assert_eq!(config.prompt_model.as_deref(), Some("qwen2.5-coder:3b"));
        assert_eq!(config.fallback_model.as_deref(), Some("qwen2.5-coder:32b"));
    }

    #[test]
    fn overrides_take_precedence_over_file() {
        let file = PartialConfig {
            api: Some(LlmApi::OpenaiCompatible),
            base_url: Some("http://localhost:11434/v1".into()),
            model: Some("file-model".into()),
            prompt_model: Some("file-prompt".into()),
            fallback_model: Some("file-fallback".into()),
            ..PartialConfig::default()
        };
        let config = LlmConfig::resolve(
            file,
            PartialConfig {
                model: Some("environment-model".into()),
                prompt_model: Some("environment-prompt".into()),
                fallback_model: Some("environment-fallback".into()),
                ..PartialConfig::default()
            },
            LlmConfigOverrides {
                model: Some("cli-model".into()),
                prompt_model: Some("cli-prompt".into()),
                fallback_model: Some("cli-fallback".into()),
                ..LlmConfigOverrides::default()
            },
        )
        .unwrap();

        assert_eq!(config.model, "cli-model");
        assert_eq!(config.prompt_model.as_deref(), Some("cli-prompt"));
        assert_eq!(config.fallback_model.as_deref(), Some("cli-fallback"));
    }

    #[test]
    fn rejects_openrouter_provider_on_generic_api() {
        let error = LlmConfig::resolve(
            PartialConfig {
                api: Some(LlmApi::OpenaiCompatible),
                base_url: Some("https://example.test/v1".into()),
                model: Some("model".into()),
                provider: Some("deepinfra".into()),
                ..PartialConfig::default()
            },
            PartialConfig::default(),
            LlmConfigOverrides::default(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("OpenRouter-only"));
    }

    #[test]
    fn debug_output_never_contains_api_keys() {
        let config = LlmConfig {
            api: LlmApi::OpenaiCompatible,
            base_url: "https://example.test/v1".into(),
            api_key: Some("super-secret-key".into()),
            model: "model".into(),
            prompt_model: None,
            fallback_model: None,
            provider: None,
        };
        let overrides = LlmConfigOverrides {
            api_key: Some("another-secret-key".into()),
            ..LlmConfigOverrides::default()
        };

        assert!(!format!("{config:?}").contains("super-secret-key"));
        assert!(!format!("{overrides:?}").contains("another-secret-key"));
    }
}
