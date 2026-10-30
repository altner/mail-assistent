use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{self, pki_types::ServerName};

use super::session::ReadOnlySession;
use crate::error::{Error, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Baut die TLS-Konfiguration: rustls mit `ring`, Mozilla-Wurzelzertifikate.
fn tls_connector() -> Result<TlsConnector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| Error::Tls(e.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Verbindet per implizitem TLS (in der Regel Port 993) und meldet sich an.
pub async fn connect(host: &str, port: u16, username: &str, password: &str) -> Result<ReadOnlySession> {
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| Error::Invalid(format!("ungültiger Servername: {host}")))?;

    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| Error::Other(format!("Zeitüberschreitung beim Verbinden mit {host}:{port}")))??;
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, tls_connector()?.connect(server_name, tcp))
        .await
        .map_err(|_| Error::Tls("Zeitüberschreitung beim TLS-Handshake".into()))?
        .map_err(|e| Error::Tls(e.to_string()))?;

    let mut client = async_imap::Client::new(tls);
    client
        .read_response()
        .await?
        .ok_or_else(|| Error::Other("Server hat keine Begrüßung gesendet".into()))?;

    let session = client
        .login(username, password)
        .await
        .map_err(|(e, _)| Error::Other(format!("Anmeldung fehlgeschlagen: {e}")))?;
    Ok(ReadOnlySession::new(session))
}
