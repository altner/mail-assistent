//! Gemeinsame TLS-Konfiguration für IMAP und HTTP: rustls mit `ring` und den
//! Mozilla-Wurzelzertifikaten (webpki-roots). Kein OpenSSL, kein aws-lc.

use std::sync::Arc;

use tokio_rustls::rustls;

use crate::error::{Error, Result};

pub fn client_config() -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| Error::Tls(e.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth())
}
