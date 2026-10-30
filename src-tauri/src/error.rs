use serde::{Serialize, Serializer};

/// Gemeinsamer Fehlertyp für alle Module. Wird für das Frontend als
/// lesbarer Text serialisiert.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Datenbank: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("IMAP: {0}")]
    Imap(#[from] async_imap::error::Error),
    #[error("Dateisystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("Schlüsselbund: {0}")]
    Secret(#[from] keyring::Error),
    #[error("KI: {0}")]
    Ai(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("Mail konnte nicht gelesen werden: {0}")]
    Parse(String),
    #[error("Ungültige Eingabe: {0}")]
    Invalid(String),
    #[error("Nicht gefunden: {0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
