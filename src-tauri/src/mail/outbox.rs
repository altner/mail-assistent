//! Der Weg vom bestätigten Entwurf zur gesendeten Mail: Wartezeit mit
//! „Rückgängig“, Versand, lokale Ablage. Am IMAP-Server wird nichts geschrieben.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::fingerprint::fingerprint;
use super::parse::parse;
use super::store::store_eml;
use crate::db::folders::{self, Folder};
use crate::db::messages::{self, NewMessage};
use crate::db::{contacts, settings};
use crate::error::{Error, Result};
use crate::paths::DataPaths;

/// Pseudo-Ordner je Konto für lokal abgelegte gesendete Mails.
pub const LOCAL_SENT_PATH: &str = "[Lokal]/Gesendet";

pub const SETTING_DELAY: &str = "send.delay_seconds";
pub const ALLOWED_DELAYS: &[u32] = &[0, 10, 30, 60];

pub fn delay_seconds(conn: &Connection) -> Result<u32> {
    Ok(settings::get(conn, SETTING_DELAY)?
        .and_then(|v| v.parse().ok())
        .filter(|d| ALLOWED_DELAYS.contains(d))
        .unwrap_or(0))
}

pub fn set_delay_seconds(conn: &Connection, delay: u32) -> Result<()> {
    if !ALLOWED_DELAYS.contains(&delay) {
        return Err(Error::Invalid(format!(
            "Verzögerung muss eine von {ALLOWED_DELAYS:?} Sekunden sein"
        )));
    }
    settings::set(conn, SETTING_DELAY, Some(&delay.to_string()))
}

/// Liefert den lokalen Gesendet-Ordner des Kontos, legt ihn bei Bedarf an.
pub fn local_sent_folder(conn: &Connection, account_id: i64) -> Result<Folder> {
    let folder = folders::upsert(conn, account_id, LOCAL_SENT_PATH)?;
    conn.execute(
        "UPDATE imap_folders SET is_local = 1 WHERE id = ?1",
        [folder.id],
    )?;
    Ok(folder)
}

/// Legt eine gesendete Mail im Archiv ab (Datei + Zeile, als gelesen) und
/// zählt die Empfänger im Adressbuch. Liefert die Mail-ID.
pub fn archive_sent(
    conn: &mut Connection,
    paths: &DataPaths,
    account_id: i64,
    raw: &[u8],
) -> Result<i64> {
    let parsed = parse(raw)?;
    let fp = fingerprint(&parsed);
    let tx = conn.transaction()?;
    let folder = local_sent_folder(&tx, account_id)?;
    let uid = folder.last_uid + 1;
    let raw_path = store_eml(paths, account_id, &fp, raw)?;
    let to_addrs =
        (!parsed.to.is_empty()).then(|| serde_json::to_string(&parsed.to).unwrap_or_default());
    for r in &parsed.to {
        if let Some(addr) = &r.address {
            contacts::record_seen(&tx, addr, r.name.as_deref(), parsed.date.as_deref())?;
        }
    }
    let new = NewMessage {
        account_id,
        folder_id: folder.id,
        uid,
        message_id: parsed.message_id,
        fingerprint: fp.clone(),
        in_reply_to: parsed.in_reply_to,
        refs: parsed.references,
        from_addr: parsed.from_addr,
        from_name: parsed.from_name,
        to_addrs,
        subject: parsed.subject,
        date: parsed.date,
        body_text: parsed.body_text,
        raw_path,
        is_read: true,
    };
    let id = match messages::insert(&tx, &new)? {
        Some(id) => id,
        None => messages::find_by_fingerprint(&tx, account_id, &fp)?
            .map(|e| e.id)
            .ok_or_else(|| Error::Other("Gesendete Mail konnte nicht abgelegt werden".into()))?,
    };
    folders::set_last_uid(&tx, folder.id, uid)?;
    tx.commit()?;
    Ok(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SendPhase {
    Waiting,
    Sending,
    Sent,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendStatus {
    pub draft_id: i64,
    pub phase: SendPhase,
    pub seconds_left: u32,
    pub error: Option<String>,
    /// ID der lokal abgelegten Mail nach dem Versand
    pub message_id: Option<i64>,
}

/// Wartet `delay` Sekunden und meldet jede Sekunde den Rest. `false`, wenn
/// in der Zeit abgebrochen wurde.
pub async fn wait_with_undo(
    draft_id: i64,
    delay: u32,
    cancel: &Arc<AtomicBool>,
    report: &(dyn Fn(SendStatus) + Send + Sync),
) -> bool {
    let mut left = delay;
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        report(SendStatus {
            draft_id,
            phase: SendPhase::Waiting,
            seconds_left: left,
            error: None,
            message_id: None,
        });
        // In kleinen Schritten schlafen, damit „Rückgängig“ sofort greift.
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
        }
        left -= 1;
    }
    !cancel.load(Ordering::Relaxed)
}

/// Entwurfsdateien (angehängte Dateien) liegen unter `drafts/<id>/`.
pub fn draft_dir(paths: &DataPaths, draft_id: i64) -> std::path::PathBuf {
    paths.resolve(&format!("drafts/{draft_id}"))
}

pub fn remove_draft_files(paths: &DataPaths, draft_id: i64) {
    let dir = draft_dir(paths, draft_id);
    if dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&dir)
    {
        log::warn!("Entwurfsdateien {} nicht entfernt: {e}", dir.display());
    }
}

/// Alle Mails eines lokalen Ordners sind „von uns“; beim Löschen eines
/// Kontos o. ä. hilfreich, hier für Tests.
pub fn is_local_folder(conn: &Connection, folder_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT is_local = 1 FROM imap_folders WHERE id = ?1",
        params![folder_id],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn setup() -> (Db, DataPaths, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::init(dir.path().to_path_buf()).unwrap();
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'me@x.de');",
            )?;
            Ok(())
        })
        .unwrap();
        (db, paths, dir)
    }

    #[test]
    fn archive_sent_creates_local_folder_and_counts_contacts() {
        let (db, paths, _dir) = setup();
        let raw = b"From: Ich <me@x.de>\r\nTo: Tom <tom@c.de>\r\nSubject: Hallo\r\nDate: Mon, 27 Oct 2026 09:15:00 +0000\r\nMessage-ID: <a@x.de>\r\n\r\nText";
        db.with(|c| {
            let id = archive_sent(c, &paths, 1, raw)?;
            let m = messages::get(c, id)?;
            assert_eq!(m.summary.folder_path, LOCAL_SENT_PATH);
            assert!(m.summary.is_read);
            assert_eq!(m.summary.subject.as_deref(), Some("Hallo"));
            let folder = local_sent_folder(c, 1)?;
            assert!(is_local_folder(c, folder.id)?);
            assert_eq!(folder.last_uid, 1);
            assert!(paths.resolve(&m.raw_path).exists());
            let tom = contacts::suggest(c, "tom", 5)?;
            assert_eq!(tom[0].seen_count, 1);

            // Zweiter Versand: neue UID, gleicher Ordner. Gleiche Mail: keine Dublette.
            let raw2 =
                b"From: me@x.de\r\nTo: tom@c.de\r\nSubject: Zwei\r\nMessage-ID: <b@x.de>\r\n\r\nT";
            let id2 = archive_sent(c, &paths, 1, raw2)?;
            assert_ne!(id, id2);
            assert_eq!(local_sent_folder(c, 1)?.last_uid, 2);
            assert_eq!(archive_sent(c, &paths, 1, raw)?, id);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delay_setting_is_validated() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            assert_eq!(delay_seconds(c)?, 0);
            set_delay_seconds(c, 30)?;
            assert_eq!(delay_seconds(c)?, 30);
            assert!(set_delay_seconds(c, 7).is_err());
            settings::set(c, SETTING_DELAY, Some("999"))?;
            assert_eq!(delay_seconds(c)?, 0, "ungültiger Wert fällt auf 0 zurück");
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn wait_with_undo_can_be_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let seen = std::sync::Mutex::new(Vec::new());
        let report = |s: SendStatus| seen.lock().unwrap().push(s.seconds_left);

        assert!(wait_with_undo(1, 0, &cancel, &report).await);

        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        assert!(!wait_with_undo(1, 10, &cancel, &report).await);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(seen.lock().unwrap().first(), Some(&10));
    }
}
