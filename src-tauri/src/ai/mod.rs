//! KI-Vorschläge über einen lokalen Server mit Anthropic-Messages-Schnittstelle
//! (`/v1/messages`): oMLX, Splash oder Ollama.
//!
//! Regel: Die KI schlägt nur vor. Ergebnisse werden als `classifications` /
//! `message_tags` mit `source = 'ai'` und `status = 'pending'` gespeichert und
//! erst im Review bestätigt, geändert oder verworfen.

pub mod client;
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
    Custom,
}

impl Preset {
    pub fn default_url(self) -> &'static str {
        match self {
            Preset::Ollama => "http://localhost:11434",
            Preset::Omlx | Preset::Splash => "http://127.0.0.1:8000",
            Preset::Custom => "",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Preset::Ollama => "ollama",
            Preset::Omlx => "omlx",
            Preset::Splash => "splash",
            Preset::Custom => "custom",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "omlx" => Preset::Omlx,
            "splash" => Preset::Splash,
            "custom" => Preset::Custom,
            _ => Preset::Ollama,
        }
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
}
