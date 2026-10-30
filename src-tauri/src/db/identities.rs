//! Absenderadressen (Identitäten) je Konto, mit Signatur.

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub email: String,
    pub signature_text: Option<String>,
    pub signature_html: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInput {
    pub name: String,
    pub email: String,
    pub signature_text: Option<String>,
    pub signature_html: Option<String>,
}

impl IdentityInput {
    fn validated(&self) -> Result<(String, String, Option<String>, Option<String>)> {
        let name = self.name.split_whitespace().collect::<Vec<_>>().join(" ");
        let email = self.email.trim().to_lowercase();
        if name.is_empty() {
            return Err(Error::Invalid("Name fehlt".into()));
        }
        if !looks_like_email(&email) {
            return Err(Error::Invalid(format!(
                "„{email}“ ist keine gültige Adresse"
            )));
        }
        let clean = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(String::from)
        };
        Ok((
            name,
            email,
            clean(&self.signature_text),
            clean(&self.signature_html),
        ))
    }
}

/// Grobe Prüfung: genau ein `@`, davor und dahinter etwas, Domain mit Punkt.
pub fn looks_like_email(s: &str) -> bool {
    let mut parts = s.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !s.contains(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == ',')
        }
        _ => false,
    }
}

const SELECT: &str =
    "SELECT id, account_id, name, email, signature_text, signature_html, is_default
    FROM identities";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    Ok(Identity {
        id: r.get(0)?,
        account_id: r.get(1)?,
        name: r.get(2)?,
        email: r.get(3)?,
        signature_text: r.get(4)?,
        signature_html: r.get(5)?,
        is_default: r.get::<_, i64>(6)? == 1,
    })
}

/// Alle Identitäten (ohne Konto-Filter: aller Konten), Standard zuerst.
pub fn list(conn: &Connection, account_id: Option<i64>) -> Result<Vec<Identity>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE ?1 IS NULL OR account_id = ?1 ORDER BY account_id, is_default DESC, id"
    ))?;
    let rows = stmt.query_map([account_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Identity> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Identität {id}")))
}

/// Standard-Identität des Kontos; ohne Standard die erste.
pub fn default_for(conn: &Connection, account_id: i64) -> Result<Option<Identity>> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE account_id = ?1 ORDER BY is_default DESC, id LIMIT 1"),
            [account_id],
            from_row,
        )
        .optional()?)
}

fn unique(e: rusqlite::Error, email: &str) -> Error {
    match e.sqlite_error_code() {
        Some(ErrorCode::ConstraintViolation) => {
            Error::Invalid(format!("„{email}“ gibt es in diesem Konto schon"))
        }
        _ => e.into(),
    }
}

/// Legt eine Identität an; die erste eines Kontos wird Standard.
pub fn create(conn: &Connection, account_id: i64, input: &IdentityInput) -> Result<i64> {
    let (name, email, sig_text, sig_html) = input.validated()?;
    let first = default_for(conn, account_id)?.is_none();
    conn.execute(
        "INSERT INTO identities (account_id, name, email, signature_text, signature_html, is_default)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![account_id, name, email, sig_text, sig_html, first as i64],
    )
    .map_err(|e| unique(e, &email))?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, input: &IdentityInput) -> Result<()> {
    let (name, email, sig_text, sig_html) = input.validated()?;
    let changed = conn
        .execute(
            "UPDATE identities SET name = ?2, email = ?3, signature_text = ?4, signature_html = ?5
             WHERE id = ?1",
            params![id, name, email, sig_text, sig_html],
        )
        .map_err(|e| unique(e, &email))?;
    if changed == 0 {
        return Err(Error::NotFound(format!("Identität {id}")));
    }
    Ok(())
}

pub fn set_default(conn: &mut Connection, id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    let account_id: i64 = tx
        .query_row(
            "SELECT account_id FROM identities WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Identität {id}")))?;
    tx.execute(
        "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
        [account_id],
    )?;
    tx.execute("UPDATE identities SET is_default = 1 WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Löscht eine Identität; die letzte eines Kontos bleibt. Entwürfe damit
/// verlieren nur den Verweis. War sie Standard, rückt die nächste nach.
pub fn delete(conn: &mut Connection, id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    let (account_id, was_default): (i64, i64) = tx
        .query_row(
            "SELECT account_id, is_default FROM identities WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Identität {id}")))?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM identities WHERE account_id = ?1",
        [account_id],
        |r| r.get(0),
    )?;
    if count <= 1 {
        return Err(Error::Invalid(
            "Die letzte Absenderadresse eines Kontos kann nicht gelöscht werden".into(),
        ));
    }
    tx.execute(
        "UPDATE drafts SET identity_id = NULL WHERE identity_id = ?1",
        [id],
    )?;
    tx.execute("DELETE FROM identities WHERE id = ?1", [id])?;
    if was_default == 1 {
        tx.execute(
            "UPDATE identities SET is_default = 1 WHERE id =
               (SELECT id FROM identities WHERE account_id = ?1 ORDER BY id LIMIT 1)",
            [account_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn account(conn: &Connection, id: i64, username: &str) {
        conn.execute(
            "INSERT INTO accounts (id, name, imap_host, username) VALUES (?1, 'A', 'h', ?2)",
            params![id, username],
        )
        .unwrap();
    }

    fn input(name: &str, email: &str) -> IdentityInput {
        IdentityInput {
            name: name.into(),
            email: email.into(),
            signature_text: Some("  Gruß\nAdrian ".into()),
            signature_html: None,
        }
    }

    #[test]
    fn email_check() {
        assert!(looks_like_email("a.b@example.org"));
        assert!(!looks_like_email("a@b"));
        assert!(!looks_like_email("a@@b.de"));
        assert!(!looks_like_email("Name <a@b.de>"));
        assert!(!looks_like_email("a b@c.de"));
    }

    #[test]
    fn first_identity_is_default_and_last_cannot_go() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            account(c, 1, "nicht-mail");
            assert!(default_for(c, 1)?.is_none());
            let a = create(c, 1, &input("Adrian", "A@Example.org"))?;
            let b = create(c, 1, &input("Adrian privat", "p@example.org"))?;
            assert!(matches!(
                create(c, 1, &input("x", "a@example.org")),
                Err(Error::Invalid(_))
            ));
            assert!(matches!(
                create(c, 1, &input("x", "kaputt")),
                Err(Error::Invalid(_))
            ));

            let list = list(c, Some(1))?;
            assert_eq!(list[0].id, a);
            assert!(list[0].is_default);
            assert_eq!(list[0].email, "a@example.org");
            assert_eq!(list[0].signature_text.as_deref(), Some("Gruß\nAdrian"));

            set_default(c, b)?;
            assert_eq!(default_for(c, 1)?.unwrap().id, b);
            delete(c, b)?;
            assert_eq!(default_for(c, 1)?.unwrap().id, a, "Standard rückt nach");
            assert!(matches!(delete(c, a), Err(Error::Invalid(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migration_seeds_identity_from_username() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            // Die Migration ist schon gelaufen; Seed-Logik hier direkt nachstellen.
            account(c, 1, "Me@Example.org");
            c.execute_batch(
                "INSERT INTO identities (account_id, name, email, is_default)
                 SELECT id, name, lower(username), 1 FROM accounts WHERE username LIKE '%@%'
                   AND id NOT IN (SELECT account_id FROM identities);",
            )?;
            let d = default_for(c, 1)?.unwrap();
            assert_eq!(d.email, "me@example.org");
            assert_eq!(d.name, "A");
            Ok(())
        })
        .unwrap();
    }
}
