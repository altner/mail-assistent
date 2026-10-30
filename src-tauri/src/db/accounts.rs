use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub username: String,
    pub created_at: String,
    pub message_count: i64,
    pub smtp_host: Option<String>,
    pub smtp_port: u16,
}

/// Kontodaten ohne Passwort; das Passwort geht direkt in den Schlüsselbund.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub username: String,
}

impl NewAccount {
    pub fn validated(self) -> Result<Self> {
        let name = self.name.trim().to_string();
        let imap_host = self.imap_host.trim().to_ascii_lowercase();
        let username = self.username.trim().to_string();
        if name.is_empty() || imap_host.is_empty() || username.is_empty() {
            return Err(Error::Invalid(
                "Name, Server und Benutzername sind Pflicht".into(),
            ));
        }
        if imap_host.contains(|c: char| c.is_whitespace() || c == '/' || c == ':') {
            return Err(Error::Invalid(
                "Server bitte ohne Protokoll/Port angeben".into(),
            ));
        }
        if self.imap_port == 0 {
            return Err(Error::Invalid("Port fehlt".into()));
        }
        Ok(Self {
            name,
            imap_host,
            imap_port: self.imap_port,
            username,
        })
    }
}

const SELECT: &str = "SELECT a.id, a.name, a.imap_host, a.imap_port, a.username, a.created_at,
        (SELECT count(*) FROM messages m WHERE m.account_id = a.id),
        a.smtp_host, a.smtp_port
     FROM accounts a";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        name: r.get(1)?,
        imap_host: r.get(2)?,
        imap_port: r.get(3)?,
        username: r.get(4)?,
        created_at: r.get(5)?,
        message_count: r.get(6)?,
        smtp_host: r.get(7)?,
        smtp_port: r.get(8)?,
    })
}

pub fn insert(conn: &Connection, account: &NewAccount) -> Result<i64> {
    conn.execute(
        "INSERT INTO accounts (name, imap_host, imap_port, username) VALUES (?1, ?2, ?3, ?4)",
        params![
            account.name,
            account.imap_host,
            account.imap_port,
            account.username
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY a.name COLLATE NOCASE"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Account> {
    conn.query_row(&format!("{SELECT} WHERE a.id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Konto {id}")))
}
