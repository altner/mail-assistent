//! SQLite-Zugriff: Verbindung, Migrationen und Queries je Tabelle.

pub mod accounts;
pub mod folders;
pub mod messages;
pub mod migrations;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Gemeinsam genutzte Verbindung. Zugriffe sind kurz und synchron; aus
/// async-Code bitte über `tokio::task::spawn_blocking` aufrufen, wenn
/// größere Mengen geschrieben werden.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        migrations::run(&mut conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self
            .0
            .lock()
            .map_err(|_| Error::Other("Datenbank-Sperre ist beschädigt".into()))?;
        f(&mut conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table') ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn migration_creates_full_schema() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            assert_eq!(migrations::current_version(conn)?, migrations::latest_version());
            let tables = table_names(conn);
            for t in [
                "accounts",
                "attachments",
                "categories",
                "classifications",
                "imap_folders",
                "message_tags",
                "messages",
                "messages_fts",
                "tags",
                "threads",
            ] {
                assert!(tables.iter().any(|n| n == t), "Tabelle {t} fehlt");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migration_is_idempotent_on_file_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        Db::open(&path).unwrap();
        // Zweites Öffnen darf nichts erneut anwenden und nicht scheitern.
        let db = Db::open(&path).unwrap();
        db.with(|conn| {
            assert_eq!(migrations::current_version(conn)?, migrations::latest_version());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rejects_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
        assert!(migrations::run(&mut conn).is_err());
    }

    fn seed_message(conn: &Connection, subject: &str) -> i64 {
        conn.execute_batch(
            "INSERT OR IGNORE INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u');
             INSERT OR IGNORE INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, fingerprint, subject, body_text, raw_path)
             VALUES (1, 1, abs(random() % 100000), hex(randomblob(8)), ?1, 'Hallo Welt', 'x.eml')",
            [subject],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn fts_hits(conn: &Connection, query: &str) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
            [query],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn fts_follows_insert_update_delete() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let id = seed_message(conn, "Rechnung Oktober");
            assert_eq!(fts_hits(conn, "rechnung"), 1);

            conn.execute("UPDATE messages SET subject = 'Mahnung' WHERE id = ?1", [id])?;
            assert_eq!(fts_hits(conn, "rechnung"), 0);
            assert_eq!(fts_hits(conn, "mahnung"), 1);

            // uid/is_read-Updates fassen den Index nicht an
            conn.execute("UPDATE messages SET is_read = 1 WHERE id = ?1", [id])?;
            assert_eq!(fts_hits(conn, "mahnung"), 1);

            conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            assert_eq!(fts_hits(conn, "mahnung"), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn only_one_confirmed_category_per_message() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let id = seed_message(conn, "Test");
            conn.execute_batch("INSERT INTO categories (id, name) VALUES (1, 'Finanzen'), (2, 'Privat');")?;
            let insert = "INSERT INTO classifications (message_id, category_id, source, status) VALUES (?1, ?2, ?3, ?4)";
            conn.execute(insert, rusqlite::params![id, 1, "ai", "pending"])?;
            conn.execute(insert, rusqlite::params![id, 2, "ai", "pending"])?;
            conn.execute(insert, rusqlite::params![id, 1, "user", "accepted"])?;
            let second = conn.execute(insert, rusqlite::params![id, 2, "user", "changed"]);
            assert!(second.is_err(), "zweite bestätigte Kategorie muss scheitern");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let res = conn.execute(
                "INSERT INTO imap_folders (account_id, path) VALUES (42, 'INBOX')",
                [],
            );
            assert!(res.is_err());
            Ok(())
        })
        .unwrap();
    }
}
