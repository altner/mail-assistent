//! Passwörter/App-Passwörter im Schlüsselbund des Betriebssystems
//! (macOS: Keychain, Windows: Credential Manager, Linux: Secret Service).
//! Sie landen nie in der Datenbank oder in Dateien.

use keyring::Entry;

use crate::error::{Error, Result};

/// Service-Name im Schlüsselbund = Bundle-ID der App.
const SERVICE: &str = "de.adrianaltner.mailassistent";

fn entry(account_id: i64) -> Result<Entry> {
    open(&format!("imap-account-{account_id}"))
}

/// Unter Linux fehlt der Speicher, wenn kein Secret Service läuft; das soll
/// verständlich gemeldet werden statt als „NoDefaultStore“.
fn open(user: &str) -> Result<Entry> {
    Entry::new(SERVICE, user).map_err(|e| match e {
        keyring::Error::NoDefaultStore => Error::Other(NO_STORE.into()),
        e => e.into(),
    })
}

const NO_STORE: &str = "Kein Schlüsselbund verfügbar. Unter Linux braucht die App einen \
                        laufenden Secret Service, z. B. GNOME Schlüsselbund oder KWallet.";

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
/// API-Schlüssel des eigenen Servers für Entwürfe (z. B. Anthropic).
const AI_DRAFT_KEY_USER: &str = "ai-draft-api-key";

fn set_key(user: &str, key: &str) -> Result<()> {
    if key.is_empty() {
        return delete_key(user);
    }
    Ok(open(user)?.set_password(key)?)
}

/// `None`, wenn kein Schlüssel hinterlegt ist.
fn get_key(user: &str) -> Result<Option<String>> {
    match open(user)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn delete_key(user: &str) -> Result<()> {
    match open(user)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn set_ai_key(key: &str) -> Result<()> {
    set_key(AI_KEY_USER, key)
}

pub fn get_ai_key() -> Result<Option<String>> {
    get_key(AI_KEY_USER)
}

pub fn delete_ai_key() -> Result<()> {
    delete_key(AI_KEY_USER)
}

pub fn set_ai_draft_key(key: &str) -> Result<()> {
    set_key(AI_DRAFT_KEY_USER, key)
}

pub fn get_ai_draft_key() -> Result<Option<String>> {
    get_key(AI_DRAFT_KEY_USER)
}
