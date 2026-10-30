//! Kategorien und Tags. Sie existieren nur lokal und gelten für alle Konten.

use rusqlite::{Connection, ErrorCode, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: i64,
    pub name: String,
    /// Hilfetext für die KI
    pub description: Option<String>,
    /// Mails mit dieser bestätigten Kategorie
    pub confirmed_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInput {
    pub name: String,
    pub description: Option<String>,
}

impl CategoryInput {
    fn validated(&self) -> Result<(String, Option<String>)> {
        let name = clean_name(&self.name, "Kategorie")?;
        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(String::from);
        Ok((name, description))
    }
}

fn clean_name(name: &str, what: &str) -> Result<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(Error::Invalid(format!("{what}: Name fehlt")));
    }
    Ok(name)
}

/// Doppelte Namen als verständlichen Fehler melden.
fn unique(e: rusqlite::Error, name: &str) -> Error {
    match e.sqlite_error_code() {
        Some(ErrorCode::ConstraintViolation) => Error::Invalid(format!("„{name}“ gibt es schon")),
        _ => e.into(),
    }
}

/// In Anlage-Reihenfolge, damit die Tastenkürzel 1–9 stabil bleiben.
pub fn list(conn: &Connection) -> Result<Vec<Category>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.description,
                (SELECT count(*) FROM classifications k
                 WHERE k.category_id = c.id AND k.status IN ('accepted', 'changed'))
         FROM categories c ORDER BY c.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Category {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
            confirmed_count: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create(conn: &Connection, input: &CategoryInput) -> Result<i64> {
    let (name, description) = input.validated()?;
    conn.execute(
        "INSERT INTO categories (name, description) VALUES (?1, ?2)",
        params![name, description],
    )
    .map_err(|e| unique(e, &name))?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, input: &CategoryInput) -> Result<()> {
    let (name, description) = input.validated()?;
    let changed = conn
        .execute(
            "UPDATE categories SET name = ?2, description = ?3 WHERE id = ?1",
            params![id, name, description],
        )
        .map_err(|e| unique(e, &name))?;
    if changed == 0 {
        return Err(Error::NotFound(format!("Kategorie {id}")));
    }
    Ok(())
}

/// Löscht eine Kategorie samt offenen und abgelehnten Vorschlägen dafür.
/// Ist sie Mails fest zugeordnet, wird nichts gelöscht.
pub fn delete(conn: &mut Connection, id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    let confirmed: i64 = tx.query_row(
        "SELECT count(*) FROM classifications
         WHERE category_id = ?1 AND status IN ('accepted', 'changed')",
        [id],
        |r| r.get(0),
    )?;
    if confirmed > 0 {
        return Err(Error::Invalid(format!(
            "Die Kategorie ist {confirmed} Mail(s) fest zugeordnet und kann nicht gelöscht werden"
        )));
    }
    tx.execute(
        "UPDATE classifications SET suggested_category_id = NULL WHERE suggested_category_id = ?1",
        [id],
    )?;
    tx.execute(
        "DELETE FROM classifications WHERE category_id = ?1 AND status IN ('pending', 'rejected')",
        [id],
    )?;
    if tx.execute("DELETE FROM categories WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound(format!("Kategorie {id}")));
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

pub fn list_tags(conn: &Connection) -> Result<Vec<Tag>> {
    let mut stmt = conn.prepare("SELECT id, name FROM tags ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| {
        Ok(Tag {
            id: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create_tag(conn: &Connection, name: &str) -> Result<i64> {
    let name = clean_name(name, "Tag")?;
    conn.execute("INSERT INTO tags (name) VALUES (?1)", [&name])
        .map_err(|e| unique(e, &name))?;
    Ok(conn.last_insert_rowid())
}

pub fn rename_tag(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = clean_name(name, "Tag")?;
    let changed = conn
        .execute("UPDATE tags SET name = ?2 WHERE id = ?1", params![id, name])
        .map_err(|e| unique(e, &name))?;
    if changed == 0 {
        return Err(Error::NotFound(format!("Tag {id}")));
    }
    Ok(())
}

/// Löscht den Tag und entfernt ihn von allen Mails.
pub fn delete_tag(conn: &mut Connection, id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM message_tags WHERE tag_id = ?1", [id])?;
    if tx.execute("DELETE FROM tags WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound(format!("Tag {id}")));
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn input(name: &str) -> CategoryInput {
        CategoryInput {
            name: name.into(),
            description: Some("  Hilfe  ".into()),
        }
    }

    #[test]
    fn starts_with_seeded_categories() {
        let db = Db::open_in_memory().unwrap();
        let list = db.with(|c| list(c)).unwrap();
        assert_eq!(list.len(), 10);
        assert_eq!(list[0].name, "Rechnungen");
        assert!(list.iter().all(|c| c.description.is_some()));
    }

    #[test]
    fn create_update_rejects_duplicates_and_blanks() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let id = create(c, &input("  Haus   und Garten "))?;
            let created = list(c)?.into_iter().find(|k| k.id == id).unwrap();
            assert_eq!(created.name, "Haus und Garten");
            assert_eq!(created.description.as_deref(), Some("Hilfe"));

            assert!(matches!(
                create(c, &input("Reisen")),
                Err(Error::Invalid(_))
            ));
            assert!(matches!(create(c, &input("   ")), Err(Error::Invalid(_))));
            assert!(matches!(
                update(c, id, &input("Reisen")),
                Err(Error::Invalid(_))
            ));
            update(c, id, &input("Garten"))?;
            assert!(list(c)?.iter().any(|k| k.name == "Garten"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_refuses_confirmed_and_cleans_suggestions() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u');
                 INSERT INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');
                 INSERT INTO messages (id, account_id, folder_id, uid, fingerprint, raw_path)
                   VALUES (1, 1, 1, 1, 'a', 'x'), (2, 1, 1, 2, 'b', 'x');",
            )?;
            let used = create(c, &input("Fest"))?;
            let loose = create(c, &input("Lose"))?;
            c.execute(
                "INSERT INTO classifications (message_id, category_id, suggested_category_id, source, status)
                 VALUES (1, ?1, ?1, 'ai', 'accepted'), (2, ?2, ?2, 'ai', 'pending')",
                params![used, loose],
            )?;

            assert!(matches!(delete(c, used), Err(Error::Invalid(_))));
            delete(c, loose)?;
            let left: i64 =
                c.query_row("SELECT count(*) FROM classifications", [], |r| r.get(0))?;
            assert_eq!(left, 1);
            assert!(!list(c)?.iter().any(|k| k.id == loose));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_crud() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            let b = create_tag(c, "wichtig")?;
            create_tag(c, "Archiv")?;
            assert!(matches!(create_tag(c, "wichtig"), Err(Error::Invalid(_))));
            rename_tag(c, b, "Wichtig")?;
            let names: Vec<_> = list_tags(c)?.into_iter().map(|t| t.name).collect();
            assert_eq!(names, ["Archiv", "Wichtig"]);
            delete_tag(c, b)?;
            assert_eq!(list_tags(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
