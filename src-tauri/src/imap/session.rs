use std::time::Duration;

use async_imap::imap_proto::NameAttribute;
use async_imap::types::{Fetch, Flag, Name};
use chrono::{SecondsFormat, Utc};
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use super::mutf7;
use crate::error::{Error, Result};
use crate::mail::ingest::RawMessage;

type Stream = TlsStream<TcpStream>;

/// Abruf-Query: `BODY.PEEK[]` lässt `\Seen` unverändert. `FLAGS` wird nur
/// gelesen, um den Gelesen-Status anzuzeigen.
pub(crate) const FETCH_QUERY: &str = "(UID FLAGS INTERNALDATE BODY.PEEK[])";

const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);

/// Rollen nach RFC 6154 (SPECIAL-USE), soweit für den Abruf relevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialUse {
    All,
    Archive,
    Drafts,
    Flagged,
    Junk,
    Sent,
    Trash,
}

#[derive(Debug, Clone)]
pub struct RemoteFolder {
    /// Originalname vom Server (modifiziertes UTF-7) – wird so gespeichert
    pub path: String,
    /// dekodiert, für Anzeige und Namensvergleich
    pub display_name: String,
    pub delimiter: Option<String>,
    pub selectable: bool,
    pub special_use: Option<SpecialUse>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExamineInfo {
    pub uidvalidity: Option<u32>,
    pub exists: u32,
}

/// Nur-Lese-Sitzung. Die innere `async_imap::Session` ist privat, damit
/// kein Aufrufer versehentlich `select`, `store`, `expunge` o. Ä. nutzt.
pub struct ReadOnlySession {
    inner: async_imap::Session<Stream>,
}

impl ReadOnlySession {
    pub(super) fn new(inner: async_imap::Session<Stream>) -> Self {
        Self { inner }
    }

    pub async fn list_folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let names: Vec<Name> = timed(async {
            self.inner.list(Some(""), Some("*")).await?.try_collect().await
        })
        .await?;
        Ok(names.iter().map(remote_folder).collect())
    }

    /// Öffnet den Ordner read-only (`EXAMINE`, nie `SELECT`).
    pub async fn examine(&mut self, path: &str) -> Result<ExamineInfo> {
        let mailbox = timed(self.inner.examine(path)).await?;
        Ok(ExamineInfo { uidvalidity: mailbox.uid_validity, exists: mailbox.exists })
    }

    /// Alle UIDs > `last_uid`, aufsteigend sortiert.
    pub async fn uids_after(&mut self, last_uid: u32) -> Result<Vec<u32>> {
        let query = format!("UID {}:*", last_uid.saturating_add(1));
        let found = timed(self.inner.uid_search(query)).await?;
        // `n:*` liefert laut RFC immer mindestens die höchste UID, auch wenn sie < n ist.
        let mut uids: Vec<u32> = found.into_iter().filter(|&u| u > last_uid).collect();
        uids.sort_unstable();
        Ok(uids)
    }

    pub async fn fetch(&mut self, uids: &[u32]) -> Result<Vec<RawMessage>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let fetches: Vec<Fetch> = timed(async {
            self.inner.uid_fetch(uid_set(uids), FETCH_QUERY).await?.try_collect().await
        })
        .await?;
        Ok(fetches.iter().filter_map(raw_message).collect())
    }

    pub async fn logout(mut self) {
        if let Err(e) = self.inner.logout().await {
            log::debug!("Logout: {e}");
        }
    }
}

async fn timed<T>(fut: impl Future<Output = async_imap::error::Result<T>>) -> Result<T> {
    tokio::time::timeout(COMMAND_TIMEOUT, fut)
        .await
        .map_err(|_| Error::Other("Zeitüberschreitung bei IMAP-Befehl".into()))?
        .map_err(Error::from)
}

fn remote_folder(name: &Name) -> RemoteFolder {
    let mut selectable = true;
    let mut special_use = None;
    for attr in name.attributes() {
        match attr {
            NameAttribute::NoSelect => selectable = false,
            NameAttribute::All => special_use = Some(SpecialUse::All),
            NameAttribute::Archive => special_use = Some(SpecialUse::Archive),
            NameAttribute::Drafts => special_use = Some(SpecialUse::Drafts),
            NameAttribute::Flagged => special_use = Some(SpecialUse::Flagged),
            NameAttribute::Junk => special_use = Some(SpecialUse::Junk),
            NameAttribute::Sent => special_use = Some(SpecialUse::Sent),
            NameAttribute::Trash => special_use = Some(SpecialUse::Trash),
            NameAttribute::Extension(ext) if ext.eq_ignore_ascii_case("\\NonExistent") => {
                selectable = false
            }
            _ => {}
        }
    }
    RemoteFolder {
        path: name.name().to_string(),
        display_name: mutf7::decode(name.name()),
        delimiter: name.delimiter().map(str::to_string),
        selectable,
        special_use,
    }
}

fn raw_message(fetch: &Fetch) -> Option<RawMessage> {
    let uid = fetch.uid?;
    let Some(body) = fetch.body() else {
        log::warn!("UID {uid}: Server hat keinen Body geliefert");
        return None;
    };
    Some(RawMessage {
        uid,
        seen: fetch.flags().any(|f| matches!(f, Flag::Seen)),
        internal_date: fetch
            .internal_date()
            .map(|d| d.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true)),
        raw: body.to_vec(),
    })
}

/// Kompakte UID-Menge, z. B. `[1,2,3,7,9,10]` → `1:3,7,9:10`.
/// Erwartet aufsteigend sortierte UIDs.
pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut parts = Vec::new();
    let mut iter = uids.iter().copied().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap_or(end);
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}:{end}") });
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_query_never_sets_seen() {
        assert!(FETCH_QUERY.contains("BODY.PEEK[]"));
        // "BODY[" ohne PEEK oder RFC822 (ohne .HEADER/.SIZE) würden \Seen setzen.
        assert!(!FETCH_QUERY.contains("BODY["));
        assert!(!FETCH_QUERY.split_whitespace().any(|t| t.trim_matches(['(', ')']) == "RFC822"));
    }

    #[test]
    fn module_uses_examine_not_select() {
        // Schutz vor Regressionen: kein Aufruf schreibender Befehle in diesem Modul.
        let src = include_str!("session.rs");
        let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
        for forbidden in [".select(", ".store(", ".uid_store(", ".expunge(", ".uid_expunge(", ".append(", ".mv(", ".uid_mv(", ".copy(", ".uid_copy(", ".delete(", ".rename(", ".create("] {
            let hits = code.matches(forbidden).count();
            // Der Test selbst enthält die Muster genau einmal in der Liste oben.
            assert_eq!(hits, 1, "verbotener Aufruf {forbidden} gefunden");
        }
        assert!(code.contains("self.inner.examine("));
    }

    #[test]
    fn compresses_uid_ranges() {
        assert_eq!(uid_set(&[5]), "5");
        assert_eq!(uid_set(&[1, 2, 3, 7, 9, 10]), "1:3,7,9:10");
        assert_eq!(uid_set(&[]), "");
    }
}
