//! Schema-Migrationen über `PRAGMA user_version`.
//!
//! Neue Migrationen werden ausschließlich hinten an `MIGRATIONS` angehängt;
//! bestehende Einträge werden nie verändert.

use rusqlite::Connection;

use crate::error::{Error, Result};

const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

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
