//! Passwörter/App-Passwörter im Schlüsselbund des Betriebssystems
//! (macOS: Keychain, Windows: Credential Manager, Linux: Secret Service).
//! Sie landen nie in der Datenbank oder in Dateien.

use keyring::Entry;

use crate::error::{Error, Result};

/// Service-Name im Schlüsselbund = Bundle-ID der App.
const SERVICE: &str = "de.adrianaltner.mailassistent";

fn entry(account_id: i64) -> Result<Entry> {
    Ok(Entry::new(SERVICE, &format!("imap-account-{account_id}"))?)
}

pub fn set_password(account_id: i64, password: &str) -> Result<()> {
    if password.is_empty() {
        return Err(Error::Invalid("Passwort darf nicht leer sein".into()));
    }
    Ok(entry(account_id)?.set_password(password)?)
}

pub fn get_password(account_id: i64) -> Result<String> {
    match entry(account_id)?.get_password() {
        Ok(pw) => Ok(pw),
        Err(keyring::Error::NoEntry) => Err(Error::NotFound(format!(
            "kein Passwort für Konto {account_id} im Schlüsselbund"
        ))),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_password(account_id: i64) -> Result<()> {
    match entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// API-Schlüssel des KI-Servers (optional, z. B. für oMLX).
const AI_KEY_USER: &str = "ai-api-key";

pub fn set_ai_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return delete_ai_key();
    }
    Ok(Entry::new(SERVICE, AI_KEY_USER)?.set_password(key)?)
}

/// `None`, wenn kein Schlüssel hinterlegt ist.
pub fn get_ai_key() -> Result<Option<String>> {
    match Entry::new(SERVICE, AI_KEY_USER)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_ai_key() -> Result<()> {
    match Entry::new(SERVICE, AI_KEY_USER)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
