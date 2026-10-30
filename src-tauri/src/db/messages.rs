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

pub fn find_by_fingerprint(conn: &Connection, account_id: i64, fingerprint: &str) -> Result<Option<Existing>> {
    Ok(conn
        .query_row(
            "SELECT id, folder_id, uid FROM messages WHERE account_id = ?1 AND fingerprint = ?2",
            params![account_id, fingerprint],
            |r| Ok(Existing { id: r.get(0)?, folder_id: r.get(1)?, uid: r.get(2)? }),
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
}

pub fn list(conn: &Connection, account_id: Option<i64>, limit: u32, offset: u32) -> Result<Vec<MessageSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.account_id, f.path, m.from_name, m.from_addr, m.subject, m.date, m.is_read
         FROM messages m JOIN imap_folders f ON f.id = m.folder_id
         WHERE ?1 IS NULL OR m.account_id = ?1
         ORDER BY m.date IS NULL, m.date DESC, m.id DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![account_id, limit, offset], |r| {
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
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count(conn: &Connection, account_id: Option<i64>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM messages WHERE ?1 IS NULL OR account_id = ?1",
        [account_id],
        |r| r.get(0),
    )?)
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

pub fn get(conn: &Connection, id: i64) -> Result<MessageDetail> {
    conn.query_row(
        "SELECT m.id, m.account_id, f.path, m.from_name, m.from_addr, m.subject, m.date, m.is_read,
                m.message_id, m.in_reply_to, m.to_addrs, m.body_text, m.raw_path
         FROM messages m JOIN imap_folders f ON f.id = m.folder_id
         WHERE m.id = ?1",
        [id],
        |r| {
            Ok(MessageDetail {
                summary: MessageSummary {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    folder_name: mutf7::decode(&r.get::<_, String>(2)?),
            folder_path: r.get(2)?,
                    from_name: r.get(3)?,
                    from_addr: r.get(4)?,
                    subject: r.get(5)?,
                    date: r.get(6)?,
                    is_read: r.get::<_, Option<bool>>(7)?.unwrap_or(false),
                },
                message_id: r.get(8)?,
                in_reply_to: r.get(9)?,
                to_addrs: r.get(10)?,
                body_text: r.get(11)?,
                raw_path: r.get(12)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("Mail {id}")))
}
