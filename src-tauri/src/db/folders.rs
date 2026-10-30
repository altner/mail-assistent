use rusqlite::{Connection, params};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub path: String,
    pub uidvalidity: Option<u32>,
    pub last_uid: u32,
}

/// Liefert den Ordner, legt ihn bei Bedarf an.
pub fn upsert(conn: &Connection, account_id: i64, path: &str) -> Result<Folder> {
    conn.execute(
        "INSERT INTO imap_folders (account_id, path) VALUES (?1, ?2)
         ON CONFLICT (account_id, path) DO NOTHING",
        params![account_id, path],
    )?;
    Ok(conn.query_row(
        "SELECT id, account_id, path, uidvalidity, COALESCE(last_uid, 0)
         FROM imap_folders WHERE account_id = ?1 AND path = ?2",
        params![account_id, path],
        |r| {
            Ok(Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                path: r.get(2)?,
                uidvalidity: r.get(3)?,
                last_uid: r.get(4)?,
            })
        },
    )?)
}

/// UIDVALIDITY hat sich geändert (oder wird erstmals gesetzt): bisherige
/// UIDs des Ordners sind ungültig. Sie werden auf `-id` gesetzt (eindeutig,
/// kollidiert nicht mit echten UIDs) und beim Neuabruf per Fingerprint
/// wieder zugeordnet. Lokale Mails bleiben in jedem Fall erhalten.
pub fn reset_for_uidvalidity(conn: &Connection, folder_id: i64, uidvalidity: Option<u32>) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE messages SET uid = -id WHERE folder_id = ?1 AND uid > 0",
        [folder_id],
    )?;
    tx.execute(
        "UPDATE imap_folders SET uidvalidity = ?2, last_uid = 0 WHERE id = ?1",
        params![folder_id, uidvalidity],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn set_last_uid(conn: &Connection, folder_id: i64, last_uid: u32) -> Result<()> {
    conn.execute(
        "UPDATE imap_folders SET last_uid = ?2 WHERE id = ?1 AND COALESCE(last_uid, 0) < ?2",
        params![folder_id, last_uid],
    )?;
    Ok(())
}
