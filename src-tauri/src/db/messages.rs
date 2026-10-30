use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::imap::mutf7;

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub account_id: i64,
    pub folder_id: i64,
    pub uid: u32,
    pub message_id: Option<String>,
    pub fingerprint: String,
    pub in_reply_to: Option<String>,
    pub refs: Option<String>,
    pub from_addr: Option<String>,
    pub from_name: Option<String>,
    /// JSON-Array `[{"name": …, "address": …}]`
    pub to_addrs: Option<String>,
    pub subject: Option<String>,
    /// RFC 3339 in UTC, damit die Textsortierung stimmt
    pub date: Option<String>,
    pub body_text: Option<String>,
    /// relativ zum App-Datenverzeichnis
    pub raw_path: String,
    pub is_read: bool,
}

/// Bereits gespeicherte Mail mit gleichem Fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Existing {
    pub id: i64,
    pub folder_id: i64,
    pub uid: i64,
}

pub fn find_by_fingerprint(
    conn: &Connection,
    account_id: i64,
    fingerprint: &str,
) -> Result<Option<Existing>> {
    Ok(conn
        .query_row(
            "SELECT id, folder_id, uid FROM messages WHERE account_id = ?1 AND fingerprint = ?2",
            params![account_id, fingerprint],
            |r| {
                Ok(Existing {
                    id: r.get(0)?,
                    folder_id: r.get(1)?,
                    uid: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Fügt die Mail ein. `None`, falls (Konto, Ordner, UID) oder (Konto,
/// Fingerprint) schon belegt ist.
pub fn insert(conn: &Connection, m: &NewMessage) -> Result<Option<i64>> {
    let changed = conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, message_id, fingerprint, in_reply_to, refs,
             from_addr, from_name, to_addrs, subject, date, body_text, raw_path, is_read)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT DO NOTHING",
        params![
            m.account_id,
            m.folder_id,
            m.uid,
            m.message_id,
            m.fingerprint,
            m.in_reply_to,
            m.refs,
            m.from_addr,
            m.from_name,
            m.to_addrs,
            m.subject,
            m.date,
            m.body_text,
            m.raw_path,
            m.is_read,
        ],
    )?;
    Ok((changed == 1).then(|| conn.last_insert_rowid()))
}

/// Ordnet eine nach UIDVALIDITY-Wechsel veraltete Mail ihrer neuen UID zu.
pub fn remap_uid(conn: &Connection, id: i64, uid: u32, is_read: bool) -> Result<()> {
    conn.execute(
        "UPDATE messages SET uid = ?2, is_read = ?3 WHERE id = ?1",
        params![id, uid, is_read],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub id: i64,
    pub account_id: i64,
    /// Originalname auf dem Server
    pub folder_path: String,
    /// dekodiert für die Anzeige
    pub folder_name: String,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub is_read: bool,
    pub thread_id: Option<i64>,
    /// bestätigte Kategorie (übernommen oder geändert), nie ein offener Vorschlag
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub to_addrs: Option<String>,
    pub body_text: Option<String>,
    pub raw_path: String,
}

/// Spalten 0–9 für [`MessageSummary`].
macro_rules! summary_columns {
    () => {
        "m.id, m.account_id, f.path, m.from_name, m.from_addr, m.subject, m.date, m.is_read,
         m.thread_id,
         (SELECT c.name FROM classifications k JOIN categories c ON c.id = k.category_id
          WHERE k.message_id = m.id AND k.status IN ('accepted', 'changed'))"
    };
}

/// Spalten 10–14 zusätzlich für [`MessageDetail`].
const SELECT_DETAIL: &str = concat!(
    "SELECT ",
    summary_columns!(),
    ", m.message_id, m.in_reply_to, m.to_addrs, m.body_text, m.raw_path
     FROM messages m JOIN imap_folders f ON f.id = m.folder_id"
);

fn summary_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: r.get(0)?,
        account_id: r.get(1)?,
        folder_name: mutf7::decode(&r.get::<_, String>(2)?),
        folder_path: r.get(2)?,
        from_name: r.get(3)?,
        from_addr: r.get(4)?,
        subject: r.get(5)?,
        date: r.get(6)?,
        is_read: r.get::<_, Option<bool>>(7)?.unwrap_or(false),
        thread_id: r.get(8)?,
        category: r.get(9)?,
    })
}

fn detail_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MessageDetail> {
    Ok(MessageDetail {
        summary: summary_from_row(r)?,
        message_id: r.get(10)?,
        in_reply_to: r.get(11)?,
        to_addrs: r.get(12)?,
        body_text: r.get(13)?,
        raw_path: r.get(14)?,
    })
}

pub fn list(
    conn: &Connection,
    account_id: Option<i64>,
    limit: u32,
    offset: u32,
) -> Result<Vec<MessageSummary>> {
    let mut stmt = conn.prepare(concat!(
        "SELECT ",
        summary_columns!(),
        " FROM messages m JOIN imap_folders f ON f.id = m.folder_id
         WHERE ?1 IS NULL OR m.account_id = ?1
         ORDER BY m.date IS NULL, m.date DESC, m.id DESC
         LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt.query_map(params![account_id, limit, offset], summary_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count(conn: &Connection, account_id: Option<i64>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM messages WHERE ?1 IS NULL OR account_id = ?1",
        [account_id],
        |r| r.get(0),
    )?)
}

/// Macht aus freier Eingabe eine FTS5-Abfrage: jedes Wort als Präfix,
/// alle Wörter müssen vorkommen. Sonderzeichen der FTS-Syntax werden
/// dadurch unschädlich. `None` bei leerer Eingabe.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Treffer aus Betreff, Absender und Text, neueste zuerst.
pub fn search(
    conn: &Connection,
    query: &str,
    account_id: Option<i64>,
    limit: u32,
    offset: u32,
) -> Result<Vec<MessageSummary>> {
    let Some(fts) = fts_query(query) else {
        return list(conn, account_id, limit, offset);
    };
    let mut stmt = conn.prepare(concat!(
        "SELECT ",
        summary_columns!(),
        " FROM messages_fts s
         JOIN messages m ON m.id = s.rowid
         JOIN imap_folders f ON f.id = m.folder_id
         WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.account_id = ?2)
         ORDER BY m.date IS NULL, m.date DESC, m.id DESC
         LIMIT ?3 OFFSET ?4"
    ))?;
    let rows = stmt.query_map(params![fts, account_id, limit, offset], summary_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn search_count(conn: &Connection, query: &str, account_id: Option<i64>) -> Result<i64> {
    let Some(fts) = fts_query(query) else {
        return count(conn, account_id);
    };
    Ok(conn.query_row(
        "SELECT count(*) FROM messages_fts s JOIN messages m ON m.id = s.rowid
         WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.account_id = ?2)",
        params![fts, account_id],
        |r| r.get(0),
    )?)
}

pub fn get(conn: &Connection, id: i64) -> Result<MessageDetail> {
    conn.query_row(
        &format!("{SELECT_DETAIL} WHERE m.id = ?1"),
        [id],
        detail_from_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("Mail {id}")))
}

/// Alle Mails einer Konversation, älteste zuerst (ohne Datum zuletzt).
pub fn list_by_thread(conn: &Connection, thread_id: i64) -> Result<Vec<MessageDetail>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_DETAIL} WHERE m.thread_id = ?1 ORDER BY m.date IS NULL, m.date, m.id"
    ))?;
    let rows = stmt.query_map([thread_id], detail_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn seed(conn: &Connection, account: i64, subject: &str, from: &str, body: &str) -> i64 {
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, name, imap_host, username) VALUES (?1, 'A', 'h', 'u')",
            [account],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO imap_folders (id, account_id, path) VALUES (?1, ?1, 'INBOX')",
            [account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, fingerprint, subject, from_name, body_text, raw_path, date)
             VALUES (?1, ?1, abs(random() % 1000000), hex(randomblob(8)), ?2, ?3, ?4, 'x', '2026-01-01T00:00:00Z')",
            params![account, subject, from, body],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn fts_query_quotes_terms_as_prefixes() {
        assert_eq!(
            fts_query("  Rechnung  märz "),
            Some("\"Rechnung\"* \"märz\"*".into())
        );
        assert_eq!(
            fts_query("a\"b (c) OR *"),
            Some("\"ab\"* \"c\"* \"OR\"*".into())
        );
        assert_eq!(fts_query("   "), None);
        assert_eq!(fts_query("(*)"), None);
    }

    #[test]
    fn search_matches_subject_sender_body_and_account() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let a = seed(c, 1, "Rechnung März", "Shop", "Betrag 42");
            let b = seed(c, 1, "Urlaub", "Anna Beispiel", "Die Rechnung kommt später");
            let other = seed(c, 2, "Rechnung", "Fremd", "x");

            let ids = |q: &str, acc: Option<i64>| -> Vec<i64> {
                search(c, q, acc, 50, 0)
                    .unwrap()
                    .iter()
                    .map(|m| m.id)
                    .collect()
            };
            let mut hits = ids("rechn", None);
            hits.sort();
            assert_eq!(hits, vec![a, b, other]);
            assert_eq!(ids("rechnung betrag", None), vec![a]);
            assert_eq!(ids("beispiel", None), vec![b]);
            assert_eq!(ids("rechnung", Some(2)), vec![other]);
            assert_eq!(search_count(c, "rechnung", Some(1))?, 2);
            assert!(
                ids("(((", None).len() == 3,
                "nur Sonderzeichen = keine Einschränkung"
            );
            assert!(ids("gibtsnicht", None).is_empty());
            Ok(())
        })
        .unwrap();
    }
}
