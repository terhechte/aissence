use std::path::PathBuf;

use thiserror::Error;

/// Errors produced while analyzing a folder.
#[derive(Debug, Error)]
pub enum Error {
    /// The folder passed to [`crate::analyze`] does not exist.
    #[error("folder not found: {0}")]
    NotFound(PathBuf),

    /// A git operation failed.
    ///
    /// `gix` surfaces a large family of distinct error types depending on the
    /// operation; we flatten them into a single message-bearing variant so the
    /// public API stays small and stable.
    #[error("git error: {0}")]
    Git(String),

    /// An I/O error occurred while reading session files.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A required environment variable was not set (e.g. `OPENROUTER_API_KEY`).
    #[error("missing environment variable: {0}")]
    MissingEnv(&'static str),

    /// LLM configuration could not be loaded or validated.
    #[error("llm configuration error: {0}")]
    LlmConfig(String),

    /// The configured LLM client or a summarization request failed.
    #[error("llm error: {0}")]
    Llm(String),
}
