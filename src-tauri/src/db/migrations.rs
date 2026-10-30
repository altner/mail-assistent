//! Schema-Migrationen über `PRAGMA user_version`.
//!
//! Neue Migrationen werden ausschließlich hinten an `MIGRATIONS` angehängt;
//! bestehende Einträge werden nie verändert.

use rusqlite::Connection;

use crate::error::{Error, Result};

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_threads.sql"),
    include_str!("migrations/0003_ai.sql"),
    include_str!("migrations/0004_send.sql"),
];

pub fn current_version(conn: &Connection) -> Result<usize> {
    let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(v as usize)
}

pub fn latest_version() -> usize {
    MIGRATIONS.len()
}

pub fn run(conn: &mut Connection) -> Result<()> {
    let current = current_version(conn)?;
    if current > MIGRATIONS.len() {
        return Err(Error::Other(format!(
            "Datenbank hat Schema-Version {current}, diese App kennt nur bis {}",
            MIGRATIONS.len()
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        log::info!("Migration {} angewendet", index + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eine Datenbank mit Schema 1 und vorhandener Mail lässt sich auf den
    /// neuesten Stand bringen, ohne dass Daten verloren gehen.
    #[test]
    fn upgrades_existing_v1_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute_batch(
            "INSERT INTO accounts (id, name, imap_host, imap_port, username) VALUES (1, 'a', 'h', 993, 'u');
             INSERT INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');
             INSERT INTO messages (account_id, folder_id, uid, fingerprint, raw_path, subject)
               VALUES (1, 1, 7, 'fp', 'x.eml', 'Hallo');",
        )
        .unwrap();

        run(&mut conn).unwrap();

        assert_eq!(current_version(&conn).unwrap(), latest_version());
        let subject: String = conn
            .query_row("SELECT subject FROM messages WHERE uid = 7", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(subject, "Hallo");
        let (count, unread): (i64, i64) = conn
            .query_row(
                "INSERT INTO threads (account_id, subject_norm) VALUES (1, 'x')
                 RETURNING message_count, unread_count",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((count, unread), (0, 0));
    }
}
