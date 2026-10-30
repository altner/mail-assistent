//! Übernahme abgerufener Rohmails ins Archiv (transportunabhängig).

use rusqlite::Connection;
use serde::Serialize;

use super::fingerprint::{fingerprint, fingerprint_raw};
use super::parse::{ParsedMail, parse};
use super::store::store_eml;
use crate::db::folders::{self, Folder};
use crate::db::messages::{self, NewMessage};
use crate::error::Result;
use crate::paths::DataPaths;

/// Eine vom Server gelesene Mail.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub uid: u32,
    /// `\Seen`-Flag, nur gelesen
    pub seen: bool,
    /// INTERNALDATE als RFC 3339 (Ersatz, wenn der `Date`-Header fehlt)
    pub internal_date: Option<String>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IngestStats {
    pub inserted: u32,
    pub duplicates: u32,
    pub remapped: u32,
    pub unparsable: u32,
}

impl std::ops::AddAssign for IngestStats {
    fn add_assign(&mut self, o: Self) {
        self.inserted += o.inserted;
        self.duplicates += o.duplicates;
        self.remapped += o.remapped;
        self.unparsable += o.unparsable;
    }
}

/// Übernimmt einen Block in einer Transaktion und setzt danach `last_uid`,
/// damit ein abgebrochener Abruf genau hier weitermacht.
pub fn ingest_batch(
    conn: &mut Connection,
    paths: &DataPaths,
    folder: &Folder,
    batch: &[RawMessage],
) -> Result<IngestStats> {
    let tx = conn.transaction()?;
    let mut stats = IngestStats::default();

    for msg in batch {
        let (parsed, fp) = match parse(&msg.raw) {
            Ok(p) => {
                let fp = fingerprint(&p);
                (p, fp)
            }
            Err(e) => {
                log::warn!("UID {} in {}: {e}; Rohdaten werden trotzdem archiviert", msg.uid, folder.path);
                stats.unparsable += 1;
                (ParsedMail::default(), fingerprint_raw(&msg.raw))
            }
        };

        match messages::find_by_fingerprint(&tx, folder.account_id, &fp)? {
            // Nach UIDVALIDITY-Wechsel: bekannte Mail bekommt ihre neue UID.
            Some(e) if e.folder_id == folder.id && e.uid < 0 => {
                messages::remap_uid(&tx, e.id, msg.uid, msg.seen)?;
                stats.remapped += 1;
            }
            Some(_) => stats.duplicates += 1,
            None => {
                let raw_path = store_eml(paths, folder.account_id, &fp, &msg.raw)?;
                let to_addrs = (!parsed.to.is_empty())
                    .then(|| serde_json::to_string(&parsed.to).unwrap_or_default());
                let new = NewMessage {
                    account_id: folder.account_id,
                    folder_id: folder.id,
                    uid: msg.uid,
                    date: parsed.date.or_else(|| msg.internal_date.clone()),
                    message_id: parsed.message_id,
                    fingerprint: fp,
                    in_reply_to: parsed.in_reply_to,
                    refs: parsed.references,
                    from_addr: parsed.from_addr,
                    from_name: parsed.from_name,
                    to_addrs,
                    subject: parsed.subject,
                    body_text: parsed.body_text,
                    raw_path,
                    is_read: msg.seen,
                };
                match messages::insert(&tx, &new)? {
                    Some(_) => stats.inserted += 1,
                    None => stats.duplicates += 1,
                }
            }
        }
    }

    if let Some(max_uid) = batch.iter().map(|m| m.uid).max() {
        folders::set_last_uid(&tx, folder.id, max_uid)?;
    }
    tx.commit()?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn raw(id: &str, subject: &str) -> Vec<u8> {
        format!(
            "From: Sender <s@example.org>\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
             Date: Mon, 1 Jan 2024 10:00:00 +0100\r\nMessage-ID: <{id}>\r\n\r\nText {subject}\r\n"
        )
        .into_bytes()
    }

    fn msg(uid: u32, id: &str) -> RawMessage {
        RawMessage { uid, seen: uid % 2 == 0, internal_date: None, raw: raw(id, &format!("Mail {uid}")) }
    }

    fn setup() -> (Db, DataPaths, tempfile::TempDir, Folder, Folder) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::init(dir.path().to_path_buf()).unwrap();
        let db = Db::open_in_memory().unwrap();
        let (inbox, archive) = db
            .with(|c| {
                c.execute("INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u')", [])?;
                Ok((folders::upsert(c, 1, "INBOX")?, folders::upsert(c, 1, "Archiv")?))
            })
            .unwrap();
        (db, paths, dir, inbox, archive)
    }

    #[test]
    fn inserts_stores_eml_and_advances_last_uid() {
        let (db, paths, _dir, inbox, _) = setup();
        let stats = db.with(|c| ingest_batch(c, &paths, &inbox, &[msg(3, "a@x"), msg(4, "b@x")])).unwrap();
        assert_eq!(stats.inserted, 2);

        db.with(|c| {
            let f = folders::upsert(c, 1, "INBOX")?;
            assert_eq!(f.last_uid, 4);
            let list = messages::list(c, Some(1), 10, 0)?;
            assert_eq!(list.len(), 2);
            let detail = messages::get(c, list[0].id)?;
            assert!(paths.resolve(&detail.raw_path).is_file());
            assert_eq!(detail.summary.date.as_deref(), Some("2024-01-01T09:00:00Z"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_in_other_folder_is_skipped() {
        let (db, paths, _dir, inbox, archive) = setup();
        db.with(|c| ingest_batch(c, &paths, &inbox, &[msg(1, "same@x")])).unwrap();
        let stats = db.with(|c| ingest_batch(c, &paths, &archive, &[msg(9, "same@x")])).unwrap();
        assert_eq!(stats, IngestStats { duplicates: 1, ..Default::default() });
        assert_eq!(db.with(|c| messages::count(c, None)).unwrap(), 1);
        // last_uid des zweiten Ordners rückt trotzdem vor
        assert_eq!(db.with(|c| folders::upsert(c, 1, "Archiv")).unwrap().last_uid, 9);
    }

    #[test]
    fn uidvalidity_reset_remaps_known_mails() {
        let (db, paths, _dir, inbox, _) = setup();
        db.with(|c| ingest_batch(c, &paths, &inbox, &[msg(1, "a@x"), msg(2, "b@x")])).unwrap();
        db.with(|c| folders::reset_for_uidvalidity(c, inbox.id, Some(77))).unwrap();

        // Server hat neu nummeriert: UID 2 ist jetzt die Mail, die vorher UID 1 hatte.
        let stats = db
            .with(|c| ingest_batch(c, &paths, &inbox, &[msg(2, "a@x"), msg(5, "new@x")]))
            .unwrap();
        assert_eq!(stats.remapped, 1);
        assert_eq!(stats.inserted, 1);

        db.with(|c| {
            let uids: Vec<i64> = c
                .prepare("SELECT uid FROM messages ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            // b@x ist auf dem Server weg, bleibt aber lokal (mit negativer UID)
            assert_eq!(uids[0], 2);
            assert!(uids[1] < 0);
            assert_eq!(uids[2], 5);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unparsable_mail_is_still_archived() {
        let (db, paths, _dir, inbox, _) = setup();
        let bad = RawMessage { uid: 1, seen: false, internal_date: None, raw: Vec::new() };
        let stats = db.with(|c| ingest_batch(c, &paths, &inbox, &[bad])).unwrap();
        assert_eq!(stats.unparsable, 1);
        assert_eq!(stats.inserted, 1);
    }
}
