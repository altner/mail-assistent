//! Adressbuch: aus Absendern und Empfängern des Archivs gesammelt oder von
//! Hand angelegt. Nur lokal.

use rusqlite::{Connection, ErrorCode, params};
use serde::{Deserialize, Serialize};

use super::identities::looks_like_email;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: i64,
    pub email: String,
    pub name: Option<String>,
    pub source: String,
    pub seen_count: i64,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInput {
    pub email: String,
    pub name: Option<String>,
}

fn clean(input: &ContactInput) -> Result<(String, Option<String>)> {
    let email = input.email.trim().to_lowercase();
    if !looks_like_email(&email) {
        return Err(Error::Invalid(format!(
            "„{email}“ ist keine gültige Adresse"
        )));
    }
    let name = input
        .name
        .as_deref()
        .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|n| !n.is_empty());
    Ok((email, name))
}

const SELECT: &str = "SELECT id, email, name, source, seen_count, last_seen FROM contacts";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: r.get(0)?,
        email: r.get(1)?,
        name: r.get(2)?,
        source: r.get(3)?,
        seen_count: r.get(4)?,
        last_seen: r.get(5)?,
    })
}

/// Vorschläge beim Tippen: Treffer in Name oder Adresse, häufigste zuerst.
pub fn suggest(conn: &Connection, query: &str, limit: u32) -> Result<Vec<Contact>> {
    let q = query.trim().to_lowercase();
    let pattern = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE ?1 = '' OR email LIKE ?2 ESCAPE '\\' OR lower(name) LIKE ?2 ESCAPE '\\'
         ORDER BY seen_count DESC, last_seen DESC, email LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![q, pattern, limit], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn list(conn: &Connection) -> Result<Vec<Contact>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} ORDER BY lower(coalesce(name, email)), email"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM contacts", [], |r| r.get(0))?)
}

pub fn create(conn: &Connection, input: &ContactInput) -> Result<i64> {
    let (email, name) = clean(input)?;
    conn.execute(
        "INSERT INTO contacts (email, name, source) VALUES (?1, ?2, 'manual')",
        params![email, name],
    )
    .map_err(|e| match e.sqlite_error_code() {
        Some(ErrorCode::ConstraintViolation) => Error::Invalid(format!("„{email}“ gibt es schon")),
        _ => e.into(),
    })?;
    Ok(conn.last_insert_rowid())
}

/// Bearbeiten macht einen Kontakt zu einem manuellen: das Archiv überschreibt
/// seinen Namen dann nicht mehr.
pub fn update(conn: &Connection, id: i64, input: &ContactInput) -> Result<()> {
    let (email, name) = clean(input)?;
    let changed = conn
        .execute(
            "UPDATE contacts SET email = ?2, name = ?3, source = 'manual' WHERE id = ?1",
            params![id, email, name],
        )
        .map_err(|e| match e.sqlite_error_code() {
            Some(ErrorCode::ConstraintViolation) => {
                Error::Invalid(format!("„{email}“ gibt es schon"))
            }
            _ => e.into(),
        })?;
    if changed == 0 {
        return Err(Error::NotFound(format!("Kontakt {id}")));
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM contacts WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound(format!("Kontakt {id}")));
    }
    Ok(())
}

/// Eine gesehene Adresse zählen. Namen werden nur bei Archiv-Kontakten
/// ergänzt oder aktualisiert, und nur wenn einer mitkommt.
pub fn record_seen(
    conn: &Connection,
    email: &str,
    name: Option<&str>,
    date: Option<&str>,
) -> Result<()> {
    let email = email.trim().to_lowercase();
    if !looks_like_email(&email) {
        return Ok(());
    }
    let name = name
        .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(&email));
    conn.execute(
        "INSERT INTO contacts (email, name, source, seen_count, last_seen)
         VALUES (?1, ?2, 'archive', 1, ?3)
         ON CONFLICT (email) DO UPDATE SET
           seen_count = seen_count + 1,
           last_seen = max(coalesce(last_seen, ''), coalesce(excluded.last_seen, '')),
           name = CASE WHEN source = 'archive' AND excluded.name IS NOT NULL
                       THEN excluded.name ELSE name END",
        params![email, name, date],
    )?;
    Ok(())
}

/// Alle Absender und Empfänger des Archivs zählen (einmalig oder zum
/// Nachziehen). Eigene Identitäten werden ausgelassen.
pub fn rebuild_from_archive(conn: &mut Connection) -> Result<usize> {
    #[derive(Deserialize)]
    struct Addr {
        name: Option<String>,
        address: Option<String>,
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM contacts WHERE source = 'archive'", [])?;
    let own: Vec<String> = {
        let mut stmt = tx.prepare("SELECT email FROM identities")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = {
        let mut stmt = tx.prepare("SELECT from_addr, from_name, to_addrs, date FROM messages")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut seen = 0usize;
    for (from, from_name, to_json, date) in rows {
        let mut all: Vec<(String, Option<String>)> = Vec::new();
        if let Some(f) = from {
            all.push((f, from_name));
        }
        if let Some(json) = to_json {
            for a in serde_json::from_str::<Vec<Addr>>(&json).unwrap_or_default() {
                if let Some(addr) = a.address {
                    all.push((addr, a.name));
                }
            }
        }
        for (addr, name) in all {
            if own.iter().any(|o| o.eq_ignore_ascii_case(&addr)) {
                continue;
            }
            record_seen(&tx, &addr, name.as_deref(), date.as_deref())?;
            seen += 1;
        }
    }
    tx.commit()?;
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn record_seen_counts_and_keeps_manual_names() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            record_seen(
                c,
                "Anna@Example.org",
                Some("Anna"),
                Some("2026-01-01T00:00:00Z"),
            )?;
            record_seen(
                c,
                "anna@example.org",
                Some("Anna Beispiel"),
                Some("2026-02-01T00:00:00Z"),
            )?;
            record_seen(c, "kaputt", Some("x"), None)?;
            let all = list(c)?;
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].name.as_deref(), Some("Anna Beispiel"));
            assert_eq!(all[0].seen_count, 2);
            assert_eq!(all[0].last_seen.as_deref(), Some("2026-02-01T00:00:00Z"));

            let id = all[0].id;
            update(
                c,
                id,
                &ContactInput {
                    email: "anna@example.org".into(),
                    name: Some("Anna B.".into()),
                },
            )?;
            record_seen(c, "anna@example.org", Some("Irgendwer"), None)?;
            assert_eq!(
                list(c)?[0].name.as_deref(),
                Some("Anna B."),
                "manueller Name bleibt"
            );
            assert_eq!(list(c)?[0].seen_count, 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn suggest_matches_name_or_email_by_frequency() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            for _ in 0..3 {
                record_seen(c, "shop@example.org", Some("Shop Info"), None)?;
            }
            record_seen(c, "anna@beispiel.de", Some("Anna Beispiel"), None)?;
            create(
                c,
                &ContactInput {
                    email: "neu@example.org".into(),
                    name: None,
                },
            )?;
            assert!(matches!(
                create(
                    c,
                    &ContactInput {
                        email: "neu@example.org".into(),
                        name: None
                    }
                ),
                Err(Error::Invalid(_))
            ));
            let hits: Vec<_> = suggest(c, "exam", 10)?
                .into_iter()
                .map(|k| k.email)
                .collect();
            assert_eq!(hits, ["shop@example.org", "neu@example.org"]);
            let hits: Vec<_> = suggest(c, "ANNA", 10)?
                .into_iter()
                .map(|k| k.email)
                .collect();
            assert_eq!(hits, ["anna@beispiel.de"]);
            assert_eq!(suggest(c, "", 2)?.len(), 2);
            assert!(
                suggest(c, "100%", 10)?.is_empty(),
                "Prozent ist kein Platzhalter"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rebuild_from_archive_skips_own_identities() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'me@x.de');
                 INSERT INTO identities (account_id, name, email, is_default) VALUES (1, 'Ich', 'me@x.de', 1);
                 INSERT INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');
                 INSERT INTO messages (account_id, folder_id, uid, fingerprint, raw_path, from_addr, from_name, to_addrs, date)
                   VALUES (1, 1, 1, 'a', 'x', 'anna@b.de', 'Anna', '[{\"name\":\"Ich\",\"address\":\"me@x.de\"},{\"name\":null,\"address\":\"tom@c.de\"}]', '2026-01-01T00:00:00Z'),
                          (1, 1, 2, 'b', 'x', 'anna@b.de', NULL, '[]', '2026-03-01T00:00:00Z');",
            )?;
            let seen = rebuild_from_archive(c)?;
            assert_eq!(seen, 3);
            let all = list(c)?;
            let emails: Vec<_> = all.iter().map(|k| k.email.as_str()).collect();
            assert_eq!(emails, ["anna@b.de", "tom@c.de"]);
            let anna = &all[0];
            assert_eq!((anna.seen_count, anna.name.as_deref()), (2, Some("Anna")));
            assert_eq!(anna.last_seen.as_deref(), Some("2026-03-01T00:00:00Z"));
            // zweiter Lauf ändert nichts
            rebuild_from_archive(c)?;
            assert_eq!(count(c)?, 2);
            Ok(())
        })
        .unwrap();
    }
}
