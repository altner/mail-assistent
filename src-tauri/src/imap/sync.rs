//! Abruf eines Kontos: Ordner auflisten, je Ordner ab `last_uid` holen,
//! blockweise ins Archiv übernehmen und Fortschritt melden.

use serde::Serialize;

use super::connect::connect;
use super::session::{ReadOnlySession, RemoteFolder, SpecialUse};
use crate::db::Db;
use crate::db::accounts::Account;
use crate::db::folders::{self, Folder};
use crate::error::{Error, Result};
use crate::mail::ingest::{IngestStats, RawMessage, ingest_batch};
use crate::paths::DataPaths;

const BATCH_SIZE: usize = 50;

/// Übliche Namen von Papierkorb/Spam, falls der Server kein SPECIAL-USE meldet.
const SKIPPED_NAMES: &[&str] = &[
    "trash",
    "papierkorb",
    "deleted",
    "deleted items",
    "deleted messages",
    "gelöschte elemente",
    "gelöschte objekte",
    "junk",
    "junk e-mail",
    "junk email",
    "spam",
    "spamverdacht",
    "bulk mail",
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    Connecting,
    Folder,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub account_id: i64,
    pub phase: SyncPhase,
    pub folder: Option<String>,
    pub folder_index: usize,
    pub folder_count: usize,
    pub done: usize,
    pub total: usize,
    pub stats: IngestStats,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub account_id: i64,
    pub folders: Vec<String>,
    pub skipped_folders: Vec<String>,
    pub folder_errors: Vec<String>,
    pub stats: IngestStats,
}

/// Soll dieser Ordner archiviert werden? (Papierkorb und Spam nicht.)
pub fn should_archive(folder: &RemoteFolder) -> bool {
    if !folder.selectable {
        return false;
    }
    if matches!(
        folder.special_use,
        Some(SpecialUse::Trash | SpecialUse::Junk)
    ) {
        return false;
    }
    let leaf = match folder.delimiter.as_deref() {
        Some(d) if !d.is_empty() => folder.display_name.rsplit(d).next().unwrap_or(""),
        _ => folder.display_name.as_str(),
    };
    let leaf = normalize_name(leaf);
    !SKIPPED_NAMES.iter().any(|n| normalize_name(n) == leaf)
}

/// Kleinschreibung; Bindestrich, Unterstrich und Leerraum zählen gleich,
/// „Junk-E-Mail“, „junk e-mail“ und „Junk_E_Mail“ sind also derselbe Name.
fn normalize_name(name: &str) -> String {
    name.to_lowercase()
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reihenfolge: INBOX zuerst, „Alle Nachrichten“ (z. B. Gmail) zuletzt, damit
/// Mails möglichst ihrem eigentlichen Ordner zugeordnet werden.
fn order_key(f: &RemoteFolder) -> (u8, String) {
    let rank = if f.path.eq_ignore_ascii_case("INBOX") {
        0
    } else if f.special_use == Some(SpecialUse::All) {
        2
    } else {
        1
    };
    (rank, f.display_name.to_lowercase())
}

pub async fn sync_account(
    db: &Db,
    paths: &DataPaths,
    account: &Account,
    password: &str,
    report: &(dyn Fn(SyncProgress) + Send + Sync),
) -> Result<SyncSummary> {
    let mut progress = SyncProgress {
        account_id: account.id,
        phase: SyncPhase::Connecting,
        folder: None,
        folder_index: 0,
        folder_count: 0,
        done: 0,
        total: 0,
        stats: IngestStats::default(),
        error: None,
    };
    report(progress.clone());

    let result = async {
        let mut session = connect(
            &account.imap_host,
            account.imap_port,
            &account.username,
            password,
        )
        .await?;
        let outcome = sync_folders(&mut session, db, paths, account, &mut progress, report).await;
        session.logout().await;
        outcome
    }
    .await;

    match &result {
        Ok(summary) => {
            progress.phase = SyncPhase::Done;
            progress.stats = summary.stats;
        }
        Err(e) => {
            progress.phase = SyncPhase::Failed;
            progress.error = Some(e.to_string());
        }
    }
    report(progress);
    result
}

async fn sync_folders(
    session: &mut ReadOnlySession,
    db: &Db,
    paths: &DataPaths,
    account: &Account,
    progress: &mut SyncProgress,
    report: &(dyn Fn(SyncProgress) + Send + Sync),
) -> Result<SyncSummary> {
    let (mut wanted, skipped): (Vec<_>, Vec<_>) = session
        .list_folders()
        .await?
        .into_iter()
        .partition(should_archive);
    wanted.sort_by_key(order_key);

    let mut summary = SyncSummary {
        account_id: account.id,
        folders: wanted.iter().map(|f| f.display_name.clone()).collect(),
        skipped_folders: skipped.iter().map(|f| f.display_name.clone()).collect(),
        folder_errors: Vec::new(),
        stats: IngestStats::default(),
    };
    progress.phase = SyncPhase::Folder;
    progress.folder_count = wanted.len();

    for (index, remote) in wanted.iter().enumerate() {
        progress.folder_index = index;
        progress.folder = Some(remote.display_name.clone());
        progress.done = 0;
        progress.total = 0;
        report(progress.clone());

        match sync_folder(session, db, paths, account, remote, progress, report).await {
            Ok(stats) => summary.stats += stats,
            // Verbindungsfehler beenden den Abruf, Ordnerfehler nicht.
            Err(Error::Imap(async_imap::error::Error::No(msg))) => {
                log::warn!("Ordner {} übersprungen: {msg}", remote.display_name);
                summary
                    .folder_errors
                    .push(format!("{}: {msg}", remote.display_name));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(summary)
}

async fn sync_folder(
    session: &mut ReadOnlySession,
    db: &Db,
    paths: &DataPaths,
    account: &Account,
    remote: &RemoteFolder,
    progress: &mut SyncProgress,
    report: &(dyn Fn(SyncProgress) + Send + Sync),
) -> Result<IngestStats> {
    let mut folder = db.with(|c| folders::upsert(c, account.id, &remote.path))?;
    let info = session.examine(&remote.path).await?;

    if info.uidvalidity.is_some() && folder.uidvalidity != info.uidvalidity {
        if folder.uidvalidity.is_some() {
            log::warn!(
                "UIDVALIDITY von {} geändert ({:?} → {:?}), Ordner wird neu synchronisiert",
                remote.display_name,
                folder.uidvalidity,
                info.uidvalidity
            );
        }
        db.with(|c| folders::reset_for_uidvalidity(c, folder.id, info.uidvalidity))?;
        folder = db.with(|c| folders::upsert(c, account.id, &remote.path))?;
    }

    let mut stats = IngestStats::default();
    if info.exists == 0 {
        return Ok(stats);
    }

    let uids = session.uids_after(folder.last_uid).await?;
    progress.total = uids.len();
    report(progress.clone());

    for chunk in uids.chunks(BATCH_SIZE) {
        let batch = session.fetch(chunk).await?;
        let batch_stats = commit_batch(db, paths, &folder, batch).await?;
        stats += batch_stats;
        progress.stats += batch_stats;
        progress.done += chunk.len();
        report(progress.clone());
    }
    Ok(stats)
}

/// Schreibt Dateien und DB-Zeilen außerhalb des async-Runtimes.
async fn commit_batch(
    db: &Db,
    paths: &DataPaths,
    folder: &Folder,
    batch: Vec<RawMessage>,
) -> Result<IngestStats> {
    let (db, paths, folder) = (db.clone(), paths.clone(), folder.clone());
    tokio::task::spawn_blocking(move || db.with(|c| ingest_batch(c, &paths, &folder, &batch)))
        .await
        .map_err(|e| Error::Other(format!("Hintergrundaufgabe abgebrochen: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, special: Option<SpecialUse>) -> RemoteFolder {
        RemoteFolder {
            path: path.into(),
            display_name: super::super::mutf7::decode(path),
            delimiter: Some("/".into()),
            selectable: true,
            special_use: special,
        }
    }

    #[test]
    fn skips_trash_and_junk() {
        assert!(should_archive(&folder("INBOX", None)));
        assert!(should_archive(&folder("Archiv/2023", None)));
        assert!(should_archive(&folder("Sent", Some(SpecialUse::Sent))));
        assert!(!should_archive(&folder("Bin", Some(SpecialUse::Trash))));
        assert!(!should_archive(&folder("Werbung", Some(SpecialUse::Junk))));
        assert!(!should_archive(&folder("Papierkorb", None)));
        assert!(!should_archive(&folder("INBOX/Spam", None)));
        assert!(!should_archive(&folder("Gel&APY-schte Elemente", None)));
        assert!(!should_archive(&folder("Junk-E-Mail", None)));
        assert!(!should_archive(&folder("Deleted_Items", None)));
        assert!(should_archive(&folder("Junk-Food-Rezepte", None)));
        let mut noselect = folder("[Gmail]", None);
        noselect.selectable = false;
        assert!(!should_archive(&noselect));
    }

    #[test]
    fn inbox_first_all_mail_last() {
        let mut list = [
            folder("[Gmail]/All Mail", Some(SpecialUse::All)),
            folder("Zeitung", None),
            folder("Archiv", None),
            folder("INBOX", None),
        ];
        list.sort_by_key(order_key);
        let names: Vec<_> = list.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["INBOX", "Archiv", "Zeitung", "[Gmail]/All Mail"]);
    }
}
