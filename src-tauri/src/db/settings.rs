//! Schlüssel-Wert-Einstellungen. Keine Geheimnisse, die liegen im Schlüsselbund.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}

/// `None` löscht den Eintrag.
pub fn set(conn: &Connection, key: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?,
        None => conn.execute("DELETE FROM settings WHERE key = ?1", [key])?,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn set_get_overwrite_delete() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            assert_eq!(get(c, "ai.url")?, None);
            set(c, "ai.url", Some("http://a"))?;
            set(c, "ai.url", Some("http://b"))?;
            assert_eq!(get(c, "ai.url")?.as_deref(), Some("http://b"));
            set(c, "ai.url", None)?;
            assert_eq!(get(c, "ai.url")?, None);
            Ok(())
        })
        .unwrap();
    }
}
