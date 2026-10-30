//! KI-Vorschläge über einen lokalen Server mit Anthropic-Messages-Schnittstelle
//! (`/v1/messages`): oMLX, Splash oder Ollama.
//!
//! Regel: Die KI schlägt nur vor. Ergebnisse werden als `classifications` /
//! `message_tags` mit `source = 'ai'` und `status = 'pending'` gespeichert und
//! erst im Review bestätigt, geändert oder verworfen.

pub mod client;
pub mod draft;
pub mod prompt;
pub mod run;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::settings;
use crate::error::Result;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    /// Standard laut Vorgabe
    #[default]
    Ollama,
    Omlx,
    Splash,
    /// Anthropic-API in der Cloud: Inhalte verlassen den Rechner
    Anthropic,
    Custom,
}

impl Preset {
    pub fn default_url(self) -> &'static str {
        match self {
            Preset::Ollama => "http://localhost:11434",
            Preset::Omlx | Preset::Splash => "http://127.0.0.1:8000",
            Preset::Anthropic => client::ANTHROPIC_URL,
            Preset::Custom => "",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Preset::Ollama => "ollama",
            Preset::Omlx => "omlx",
            Preset::Splash => "splash",
            Preset::Anthropic => "anthropic",
            Preset::Custom => "custom",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "omlx" => Preset::Omlx,
            "splash" => Preset::Splash,
            "anthropic" => Preset::Anthropic,
            "custom" => Preset::Custom,
            _ => Preset::Ollama,
        }
    }
}

/// Eigenes Profil für Antwortentwürfe. Ist `separate` aus, gilt das
/// Hauptprofil (Kategorisierung) auch für Entwürfe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAiSettings {
    pub separate: bool,
    pub preset: Preset,
    pub url: String,
    pub model: Option<String>,
}

impl Default for DraftAiSettings {
    fn default() -> Self {
        Self {
            separate: false,
            preset: Preset::Anthropic,
            url: Preset::Anthropic.default_url().into(),
            model: None,
        }
    }
}

const KEY_DRAFT_SEPARATE: &str = "ai.draft.separate";
const KEY_DRAFT_PRESET: &str = "ai.draft.preset";
const KEY_DRAFT_URL: &str = "ai.draft.url";
const KEY_DRAFT_MODEL: &str = "ai.draft.model";

impl DraftAiSettings {
    pub fn load(conn: &Connection) -> Result<Self> {
        let preset = settings::get(conn, KEY_DRAFT_PRESET)?
            .map(|k| Preset::from_key(&k))
            .unwrap_or(Preset::Anthropic);
        Ok(Self {
            separate: settings::get(conn, KEY_DRAFT_SEPARATE)?.as_deref() == Some("1"),
            preset,
            url: settings::get(conn, KEY_DRAFT_URL)?.unwrap_or_else(|| preset.default_url().into()),
            model: settings::get(conn, KEY_DRAFT_MODEL)?,
        })
    }

    pub fn save(&self, conn: &Connection) -> Result<()> {
        let url = client::parse_base_url(&self.url)?;
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        settings::set(
            conn,
            KEY_DRAFT_SEPARATE,
            Some(if self.separate { "1" } else { "0" }),
        )?;
        settings::set(conn, KEY_DRAFT_PRESET, Some(self.preset.key()))?;
        settings::set(
            conn,
            KEY_DRAFT_URL,
            Some(url.as_str().trim_end_matches('/')),
        )?;
        settings::set(conn, KEY_DRAFT_MODEL, model)?;
        Ok(())
    }
}

/// Server, Modell und Schlüsselbund-Eintrag, die für Entwürfe gelten.
pub struct DraftTarget {
    pub url: String,
    pub model: Option<String>,
    /// true: Schlüssel `ai-draft-api-key`, sonst der des Hauptprofils
    pub separate_key: bool,
}

pub fn draft_target(conn: &Connection) -> Result<DraftTarget> {
    let draft = DraftAiSettings::load(conn)?;
    if draft.separate {
        Ok(DraftTarget {
            url: draft.url,
            model: draft.model,
            separate_key: true,
        })
    } else {
        let main = AiSettings::load(conn)?;
        Ok(DraftTarget {
            url: main.url,
            model: main.model,
            separate_key: false,
        })
    }
}

/// Gespeicherte KI-Einstellungen (ohne API-Schlüssel, der liegt im Schlüsselbund).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub preset: Preset,
    pub url: String,
    pub model: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            preset: Preset::default(),
            url: Preset::default().default_url().into(),
            model: None,
        }
    }
}

const KEY_PRESET: &str = "ai.preset";
const KEY_URL: &str = "ai.url";
const KEY_MODEL: &str = "ai.model";

impl AiSettings {
    pub fn load(conn: &Connection) -> Result<Self> {
        let preset = settings::get(conn, KEY_PRESET)?
            .map(|k| Preset::from_key(&k))
            .unwrap_or_default();
        Ok(Self {
            preset,
            url: settings::get(conn, KEY_URL)?.unwrap_or_else(|| preset.default_url().into()),
            model: settings::get(conn, KEY_MODEL)?,
        })
    }

    /// Prüft die URL und speichert alles.
    pub fn save(&self, conn: &Connection) -> Result<()> {
        let url = client::parse_base_url(&self.url)?;
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        settings::set(conn, KEY_PRESET, Some(self.preset.key()))?;
        settings::set(conn, KEY_URL, Some(url.as_str().trim_end_matches('/')))?;
        settings::set(conn, KEY_MODEL, model)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn settings_round_trip_with_defaults() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            assert_eq!(AiSettings::load(c)?, AiSettings::default());
            assert_eq!(AiSettings::load(c)?.url, "http://localhost:11434");

            let omlx = AiSettings {
                preset: Preset::Omlx,
                url: " http://127.0.0.1:8000/ ".into(),
                model: Some(" Qwen3.8-27B-MTP-4bit ".into()),
            };
            omlx.save(c)?;
            let loaded = AiSettings::load(c)?;
            assert_eq!(loaded.preset, Preset::Omlx);
            assert_eq!(loaded.url, "http://127.0.0.1:8000");
            assert_eq!(loaded.model.as_deref(), Some("Qwen3.8-27B-MTP-4bit"));

            let broken = AiSettings {
                url: "irgendwas".into(),
                ..loaded
            };
            assert!(broken.save(c).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn draft_profile_falls_back_to_main() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let t = draft_target(c)?;
            assert_eq!(t.url, "http://localhost:11434");
            assert!(!t.separate_key);

            DraftAiSettings {
                separate: true,
                preset: Preset::Anthropic,
                url: Preset::Anthropic.default_url().into(),
                model: Some("claude-haiku-5-5".into()),
            }
            .save(c)?;
            let t = draft_target(c)?;
            assert_eq!(t.url, "https://api.anthropic.com");
            assert_eq!(t.model.as_deref(), Some("claude-haiku-5-5"));
            assert!(t.separate_key);
            assert_eq!(DraftAiSettings::load(c)?.preset, Preset::Anthropic);
            Ok(())
        })
        .unwrap();
    }
}
