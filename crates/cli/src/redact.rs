//! Local and server-side PII/secret redaction for shareable documents.

use std::sync::{Arc, OnceLock};

use redact_core::{AnalyzerEngine, AnonymizationStrategy, AnonymizerConfig};
use regex::Regex;

use crate::{AissenceCommit, AissenceDocument, CommitSummary, SANITIZER_VERSION};

static REDACTOR: OnceLock<Redactor> = OnceLock::new();

#[derive(Clone)]
pub struct Redactor {
    engine: AnalyzerEngine,
}

impl Redactor {
    pub fn from_environment() -> Self {
        let mut engine = AnalyzerEngine::new();
        #[cfg(feature = "ner-redaction")]
        if let Ok(model) = std::env::var("AISSENCE_NER_MODEL") {
            match redact_ner::NerRecognizer::from_file(&model) {
                Ok(recognizer) => engine
                    .recognizer_registry_mut()
                    .add_recognizer(Arc::new(recognizer)),
                Err(error) => tracing_fallback(&format!(
                    "cannot load AISSENCE_NER_MODEL {model}: {error}; using pattern detection"
                )),
            }
        }
        Self { engine }
    }

    pub fn global() -> &'static Self {
        REDACTOR.get_or_init(Self::from_environment)
    }

    pub fn redact_text(&self, text: &str) -> String {
        let config = AnonymizerConfig {
            strategy: AnonymizationStrategy::Replace,
            ..Default::default()
        };
        let pii_redacted = self
            .engine
            .anonymize(text, Some("en"), &config)
            .map(|result| result.text)
            .unwrap_or_else(|_| "[REDACTION_FAILED]".to_string());
        redact_secrets(&pii_redacted)
    }

    pub fn redact_summary(&self, summary: &mut CommitSummary) {
        summary.user_requested = self.redact_text(&summary.user_requested);
        summary.ai_answered = self.redact_text(&summary.ai_answered);
        summary.ai_implemented = self.redact_text(&summary.ai_implemented);
        summary.meta = self.redact_text(&summary.meta);
        for message in &mut summary.messages {
            message.summary = message
                .summary
                .as_deref()
                .map(|value| self.redact_text(value));
            message.content = message
                .content
                .as_deref()
                .map(|value| self.redact_text(value));
            for change in &mut message.changes {
                change.file = self.redact_text(&change.file);
                change.diff = self.redact_text(&change.diff);
            }
        }
    }

    pub fn redact_commit(&self, commit: &mut AissenceCommit) {
        commit.sanitizer_version = SANITIZER_VERSION;
        commit.title = self.redact_text(&commit.title);
        commit.body = commit.body.as_deref().map(|body| self.redact_text(body));
        for file in &mut commit.files {
            file.path = self.redact_text(&file.path);
            file.previous_path = file
                .previous_path
                .as_deref()
                .map(|path| self.redact_text(path));
        }
        for message in &mut commit.source.messages {
            message.content = message
                .content
                .as_deref()
                .map(|value| self.redact_text(value));
            for change in &mut message.changes {
                change.file = self.redact_text(&change.file);
                change.diff = self.redact_text(&change.diff);
            }
        }
        if let Some(summary) = &mut commit.summary {
            self.redact_summary(summary);
        }
        commit.source_digest = commit.calculate_source_digest();
    }

    pub fn redact_document(&self, document: &mut AissenceDocument) {
        document.project.name = document
            .project
            .name
            .as_deref()
            .map(|value| self.redact_text(value));
        document.project.repository = document
            .project
            .repository
            .as_deref()
            .map(|value| self.redact_text(value));
        for commit in &mut document.commits {
            self.redact_commit(commit);
        }
    }

    pub fn document_is_clean(&self, document: &AissenceDocument) -> bool {
        let mut redacted = document.clone();
        self.redact_document(&mut redacted);
        serde_json::to_vec(document).ok() == serde_json::to_vec(&redacted).ok()
    }
}

fn redact_secrets(text: &str) -> String {
    static DIRECT: OnceLock<Vec<Regex>> = OnceLock::new();
    static ASSIGNMENT: OnceLock<Regex> = OnceLock::new();
    static CREDENTIAL_URL: OnceLock<Regex> = OnceLock::new();
    let direct = DIRECT.get_or_init(|| {
        [
            r"\bsk-[A-Za-z0-9_-]{20,}\b",
            r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            r"\bglpat-[A-Za-z0-9_-]{20,}\b",
            r"\bxox[baprs]-[A-Za-z0-9-]{16,}\b",
            r"\bnpm_[A-Za-z0-9]{20,}\b",
            r"\bAIza[0-9A-Za-z_-]{30,}\b",
            r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
            r"(?s)-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("valid secret regex"))
        .collect()
    });
    let assignment = ASSIGNMENT.get_or_init(|| {
        Regex::new(
            r#"(?i)(\b(?:api[_-]?key|access[_-]?token|auth[_-]?token|refresh[_-]?token|client[_-]?secret|aws[_-]?secret[_-]?access[_-]?key|secret|password|passwd)\b\s*[:=]\s*[\"']?)[^\s\"',;]{8,}"#,
        )
        .expect("valid assignment regex")
    });
    let mut output = text.to_string();
    for pattern in direct {
        output = pattern
            .replace_all(&output, "[REDACTED_SECRET]")
            .into_owned();
    }
    output = assignment
        .replace_all(&output, "${1}[REDACTED_SECRET]")
        .into_owned();
    let credential_url = CREDENTIAL_URL.get_or_init(|| {
        Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://[^/\s:@]+:)[^@\s/]+@")
            .expect("valid credential URL regex")
    });
    credential_url
        .replace_all(&output, "${1}[REDACTED_SECRET]@")
        .into_owned()
}

fn tracing_fallback(message: &str) {
    eprintln!("aissence redaction: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_common_api_keys_and_email_addresses() {
        let redactor = Redactor::from_environment();
        let output = redactor.redact_text(
            "email john@example.com api_key=secret-value-123456789 OpenAI sk-abcdefghijklmnopqrstuvwxyz",
        );
        assert!(!output.contains("john@example.com"));
        assert!(!output.contains("secret-value-123456789"));
        assert!(!output.contains("sk-abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn removes_tokens_and_credentials_embedded_in_urls() {
        let output = redact_secrets(
            "postgresql://alice:correct-horse-battery@db.example/test \
             eyJabcdefghij.abcdefghijk.abcdefghijkl glpat-abcdefghijklmnopqrstuvwxyz",
        );
        assert!(!output.contains("correct-horse-battery"));
        assert!(!output.contains("eyJabcdefghij"));
        assert!(!output.contains("glpat-abcdefghijklmnopqrstuvwxyz"));
    }
}
