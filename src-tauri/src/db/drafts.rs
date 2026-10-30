//! Entwürfe. Sie bleiben lokal, bis sie im Dialog bestätigt und gesendet wurden.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl DraftKind {
    fn as_str(self) -> &'static str {
        match self {
            DraftKind::New => "new",
            DraftKind::Reply => "reply",
            DraftKind::ReplyAll => "reply_all",
            DraftKind::Forward => "forward",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "reply" => DraftKind::Reply,
            "reply_all" => DraftKind::ReplyAll,
            "forward" => DraftKind::Forward,
            _ => DraftKind::New,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub name: Option<String>,
    pub address: String,
}

/// Anhang eines Entwurfs: entweder aus einer archivierten Mail (Weiterleitung)
/// oder eine Datei, die beim Anhängen in den Entwurfsordner kopiert wurde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum DraftAttachment {
    #[serde(rename_all = "camelCase")]
    Message {
        message_id: i64,
        part: usize,
        name: String,
        mime: String,
        size: usize,
    },
    #[serde(rename_all = "camelCase")]
    File {
        /// relativ zum Datenverzeichnis (`drafts/<id>/…`)
        path: String,
        name: String,
        mime: String,
        size: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: i64,
    pub account_id: i64,
    pub identity_id: Option<i64>,
    pub in_reply_to_message: Option<i64>,
    pub kind: DraftKind,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body_html: String,
    pub attachments: Vec<DraftAttachment>,
    pub created_at: String,
    pub updated_at: String,
}

/// Felder, die der Editor ändert.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftUpdate {
    pub identity_id: Option<i64>,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body_html: String,
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn parse<T: for<'de> Deserialize<'de> + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

const SELECT: &str =
    "SELECT id, account_id, identity_id, in_reply_to_message, kind, to_addrs, cc_addrs,
        bcc_addrs, subject, body_html, attachments, created_at, updated_at FROM drafts";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: r.get(0)?,
        account_id: r.get(1)?,
        identity_id: r.get(2)?,
        in_reply_to_message: r.get(3)?,
        kind: DraftKind::parse(&r.get::<_, String>(4)?),
        to: parse(&r.get::<_, String>(5)?),
        cc: parse(&r.get::<_, String>(6)?),
        bcc: parse(&r.get::<_, String>(7)?),
        subject: r.get(8)?,
        body_html: r.get(9)?,
        attachments: parse(&r.get::<_, String>(10)?),
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

pub struct NewDraft<'a> {
    pub account_id: i64,
    pub identity_id: Option<i64>,
    pub in_reply_to_message: Option<i64>,
    pub kind: DraftKind,
    pub to: &'a [Recipient],
    pub cc: &'a [Recipient],
    pub subject: &'a str,
    pub body_html: &'a str,
    pub attachments: &'a [DraftAttachment],
}

pub fn create(conn: &Connection, d: &NewDraft<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO drafts (account_id, identity_id, in_reply_to_message, kind, to_addrs, cc_addrs,
             subject, body_html, attachments)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            d.account_id,
            d.identity_id,
            d.in_reply_to_message,
            d.kind.as_str(),
            json(&d.to),
            json(&d.cc),
            d.subject,
            d.body_html,
            json(&d.attachments),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, u: &DraftUpdate) -> Result<()> {
    let changed = conn.execute(
        "UPDATE drafts SET identity_id = ?2, to_addrs = ?3, cc_addrs = ?4, bcc_addrs = ?5,
             subject = ?6, body_html = ?7, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![
            id,
            u.identity_id,
            json(&u.to),
            json(&u.cc),
            json(&u.bcc),
            u.subject,
            u.body_html
        ],
    )?;
    if changed == 0 {
        return Err(Error::NotFound(format!("Entwurf {id}")));
    }
    Ok(())
}

pub fn set_attachments(conn: &Connection, id: i64, list: &[DraftAttachment]) -> Result<()> {
    conn.execute(
        "UPDATE drafts SET attachments = ?2 WHERE id = ?1",
        params![id, json(&list)],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Draft> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Entwurf {id}")))
}

/// Zuletzt geänderte zuerst.
pub fn list(conn: &Connection) -> Result<Vec<Draft>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY updated_at DESC, id DESC"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM drafts", [], |r| r.get(0))?)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM drafts WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound(format!("Entwurf {id}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn account(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u@x.de');",
        )
        .unwrap();
    }

    fn r(address: &str) -> Recipient {
        Recipient {
            name: None,
            address: address.into(),
        }
    }

    #[test]
    fn create_update_list_delete() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            account(c);
            let att = vec![DraftAttachment::File {
                path: "drafts/1/a.pdf".into(),
                name: "a.pdf".into(),
                mime: "application/pdf".into(),
                size: 3,
            }];
            let id = create(
                c,
                &NewDraft {
                    account_id: 1,
                    identity_id: None,
                    in_reply_to_message: None,
                    kind: DraftKind::Reply,
                    to: &[r("a@b.de")],
                    cc: &[],
                    subject: "Re: Hallo",
                    body_html: "<p>Hi</p>",
                    attachments: &att,
                },
            )?;
            let d = get(c, id)?;
            assert_eq!(d.kind, DraftKind::Reply);
            assert_eq!(d.to, vec![r("a@b.de")]);
            assert_eq!(d.attachments, att);
            assert!(d.bcc.is_empty());

            update(
                c,
                id,
                &DraftUpdate {
                    identity_id: None,
                    to: vec![r("a@b.de"), r("c@d.de")],
                    cc: vec![],
                    bcc: vec![r("me@x.de")],
                    subject: "Re: Hallo!".into(),
                    body_html: "<p>Hi!</p>".into(),
                },
            )?;
            let d = get(c, id)?;
            assert_eq!(d.to.len(), 2);
            assert_eq!(d.bcc, vec![r("me@x.de")]);
            assert_eq!(d.subject, "Re: Hallo!");
            assert_eq!(count(c)?, 1);
            assert_eq!(list(c)?[0].id, id);
            delete(c, id)?;
            assert!(matches!(get(c, id), Err(Error::NotFound(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn kind_round_trip_and_json_shape() {
        for k in [
            DraftKind::New,
            DraftKind::Reply,
            DraftKind::ReplyAll,
            DraftKind::Forward,
        ] {
            assert_eq!(DraftKind::parse(k.as_str()), k);
        }
        let json = serde_json::to_string(&DraftKind::ReplyAll).unwrap();
        assert_eq!(json, "\"reply_all\"");
        let a: DraftAttachment = serde_json::from_str(
            r#"{"source":"message","messageId":4,"part":6,"name":"x.pdf","mime":"application/pdf","size":10}"#,
        )
        .unwrap();
        assert!(matches!(a, DraftAttachment::Message { part: 6, .. }));
    }
}
