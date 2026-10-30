//! Senden per SMTP mit `lettre`: nur implizites TLS (SMTPS, Port 465),
//! rustls mit ring und den Mozilla-Wurzelzertifikaten. Zugangsdaten sind die
//! des IMAP-Kontos und kommen aus dem Schlüsselbund.
//!
//! Gesendet wird ausschließlich, was [`crate::commands::send_draft`] nach der
//! Bestätigung im Dialog übergibt. Es gibt keinen anderen Weg hierher.

use std::time::Duration;

use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::error::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(60);

pub struct SmtpConfig<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}

fn transport(c: &SmtpConfig<'_>) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    if c.host.is_empty() {
        return Err(Error::Invalid(
            "Für dieses Konto ist kein SMTP-Server eingetragen".into(),
        ));
    }
    let tls = TlsParameters::builder(c.host.to_string())
        .build_rustls()
        .map_err(|e| Error::Tls(e.to_string()))?;
    Ok(
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(c.host)
            .port(c.port)
            .tls(Tls::Wrapper(tls))
            .credentials(Credentials::new(
                c.username.to_string(),
                c.password.to_string(),
            ))
            .timeout(Some(TIMEOUT))
            .build(),
    )
}

fn describe(e: lettre::transport::smtp::Error) -> Error {
    let text = e.to_string();
    let hint = if e.is_permanent() && text.contains("535") || text.to_lowercase().contains("auth") {
        " Benutzername oder Passwort wurden vom SMTP-Server abgelehnt."
    } else if e.is_timeout() {
        " Der Server hat nicht rechtzeitig geantwortet."
    } else if e.is_client() || e.is_response() {
        ""
    } else {
        " Läuft der Server, und stimmt Port 465 (SMTPS)?"
    };
    Error::Other(format!("SMTP: {text}.{hint}"))
}

/// Verbindet, meldet sich an und trennt wieder. Sendet nichts.
pub async fn test_connection(c: &SmtpConfig<'_>) -> Result<()> {
    let t = transport(c)?;
    match t.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(Error::Other(
            "SMTP: Server hat die Verbindung abgelehnt".into(),
        )),
        Err(e) => Err(describe(e)),
    }
}

/// Sendet eine fertig gebaute Nachricht.
pub async fn send(c: &SmtpConfig<'_>, message: lettre::Message) -> Result<()> {
    let t = transport(c)?;
    t.send(message).await.map(|_| ()).map_err(describe)
}
