//! Schnittstelle für die KI-Klassifizierung (Phase 3: Ollama per HTTP).
//!
//! Regel: Die KI schlägt nur vor. Ergebnisse werden als `classifications` /
//! `message_tags` mit `source = 'ai'` und `status = 'pending'` gespeichert und
//! erst im Review-UI bestätigt oder geändert.
#![allow(dead_code)]

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::error::Result;

pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub ollama_url: String,
    pub model: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self { ollama_url: DEFAULT_OLLAMA_URL.into(), model: None }
    }
}

/// Was die KI pro Mail zu sehen bekommt (gekürzt, nur Text).
#[derive(Debug, Clone)]
pub struct ClassificationInput<'a> {
    pub subject: Option<&'a str>,
    pub from: Option<&'a str>,
    pub body_excerpt: &'a str,
    /// Verfügbare Kategorien mit Hilfetext (`categories.description`)
    pub categories: &'a [(String, Option<String>)],
    pub known_tags: &'a [String],
}

/// Strukturierter Vorschlag; entspricht dem JSON, das das Modell liefern soll.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub category: Option<String>,
    pub confidence: Option<f32>,
    pub reason: Option<String>,
    pub tags: Vec<String>,
}

pub trait Classifier: Send + Sync {
    /// Modellname, wird in `classifications.model` gespeichert.
    fn model(&self) -> &str;

    fn classify(&self, input: &ClassificationInput<'_>) -> impl Future<Output = Result<Suggestion>> + Send;
}

/// Platzhalter bis Phase 3: macht keine Vorschläge.
pub struct NoopClassifier;

impl Classifier for NoopClassifier {
    fn model(&self) -> &str {
        "none"
    }

    async fn classify(&self, _input: &ClassificationInput<'_>) -> Result<Suggestion> {
        Ok(Suggestion::default())
    }
}
