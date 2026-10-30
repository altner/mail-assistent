//! Konversationen: Neuberechnung pro Konto und Abfragen für die Liste.
//!
//! Die Zuordnung selbst steckt in [`crate::mail::thread`]. Hier werden die
//! Gruppen in `threads` gespeichert. Eine Konversation behält ihre ID, solange
//! die meisten ihrer Mails zusammenbleiben, damit Verweise darauf stabil sind.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::messages::{self, MessageDetail};
use crate::error::{Error, Result};
use crate::mail::thread::{self, ThreadInput, normalize_subject};

/// Höchstens so viele Beteiligte pro Konversation in der Liste.
const MAX_PARTICIPANTS: usize = 4;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildStats {
    pub threads: usize,
    pub created: usize,
    pub removed: usize,
}

struct Row {
    input: ThreadInput,
    is_read: bool,
    thread_id: Option<i64>,
}

/// Berechnet alle Konversationen eines Kontos neu, in einer Transaktion.
pub fn rebuild(conn: &mut Connection, account_id: i64) -> Result<RebuildStats> {
    let tx = conn.transaction()?;
    let rows: Vec<Row> = {
        let mut stmt = tx.prepare(
            "SELECT id, message_id, in_reply_to, refs, subject, date, is_read, thread_id
             FROM messages WHERE account_id = ?1",
        )?;
        stmt.query_map([account_id], |r| {
            Ok(Row {
                input: ThreadInput {
                    id: r.get(0)?,
                    message_id: r.get(1)?,
                    in_reply_to: r.get(2)?,
                    refs: r.get(3)?,
                    subject: r.get(4)?,
                    date: r.get(5)?,
                },
                is_read: r.get::<_, Option<bool>>(6)?.unwrap_or(false),
                thread_id: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let inputs: Vec<ThreadInput> = rows.iter().map(|r| r.input.clone()).collect();
    let by_id: HashMap<i64, &Row> = rows.iter().map(|r| (r.input.id, r)).collect();

    let existing: HashSet<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM threads WHERE account_id = ?1")?;
        stmt.query_map([account_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };

    // Große Gruppen wählen zuerst, welche bisherige ID sie behalten.
    let mut groups = thread::group(&inputs);
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));

    let mut stats = RebuildStats::default();
    let mut claimed: HashSet<i64> = HashSet::new();
    {
        let mut update_thread = tx.prepare_cached(
            "UPDATE threads SET subject_norm = ?2, last_date = ?3, message_count = ?4, unread_count = ?5
             WHERE id = ?1",
        )?;
        let mut insert_thread = tx.prepare_cached(
            "INSERT INTO threads (account_id, subject_norm, last_date, message_count, unread_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut set_thread = tx.prepare_cached(
            "UPDATE messages SET thread_id = ?1 WHERE id = ?2 AND thread_id IS NOT ?1",
        )?;

        for group in &groups {
            let members: Vec<&Row> = group.iter().map(|id| by_id[id]).collect();
            let subject = members[0]
                .input
                .subject
                .as_deref()
                .map(|s| normalize_subject(s).display)
                .filter(|s| !s.is_empty());
            let last_date = members.iter().filter_map(|m| m.input.date.as_deref()).max();
            let count = members.len() as i64;
            let unread = members.iter().filter(|m| !m.is_read).count() as i64;

            let thread_id = match keep_id(&members, &existing, &claimed) {
                Some(id) => {
                    update_thread.execute(params![id, subject, last_date, count, unread])?;
                    id
                }
                None => {
                    insert_thread
                        .execute(params![account_id, subject, last_date, count, unread])?;
                    stats.created += 1;
                    tx.last_insert_rowid()
                }
            };
            claimed.insert(thread_id);
            for m in &members {
                set_thread.execute(params![thread_id, m.input.id])?;
            }
        }
    }

    for id in existing.difference(&claimed) {
        tx.execute("DELETE FROM threads WHERE id = ?1", [id])?;
        stats.removed += 1;
    }
    tx.commit()?;
    stats.threads = groups.len();
    Ok(stats)
}

/// Die bisherige Thread-ID, die die meisten Mails der Gruppe tragen und die
/// noch keine andere Gruppe übernommen hat (bei Gleichstand die kleinste).
fn keep_id(members: &[&Row], existing: &HashSet<i64>, claimed: &HashSet<i64>) -> Option<i64> {
    let mut votes: HashMap<i64, usize> = HashMap::new();
    for id in members.iter().filter_map(|m| m.thread_id) {
        if existing.contains(&id) && !claimed.contains(&id) {
            *votes.entry(id).or_default() += 1;
        }
    }
    votes
        .into_iter()
        .max_by(|(a_id, a), (b_id, b)| a.cmp(b).then(b_id.cmp(a_id)))
        .map(|(id, _)| id)
}

/// Konten mit Mails ohne Konversation, z. B. nach dem Update auf Schema 2.
pub fn accounts_needing_rebuild(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT account_id FROM messages WHERE thread_id IS NULL")?;
    Ok(stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: i64,
    pub account_id: i64,
    pub subject: Option<String>,
    pub last_date: Option<String>,
    pub message_count: i64,
    pub unread_count: i64,
    /// Absender in der Reihenfolge ihres ersten Auftretens
    pub participants: Vec<String>,
}

fn summary_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadSummary> {
    Ok(ThreadSummary {
        id: r.get(0)?,
        account_id: r.get(1)?,
        subject: r.get(2)?,
        last_date: r.get(3)?,
        message_count: r.get(4)?,
        unread_count: r.get(5)?,
        participants: Vec::new(),
    })
}

fn participants(conn: &Connection, thread_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT coalesce(nullif(trim(from_name), ''), from_addr) FROM messages
         WHERE thread_id = ?1 ORDER BY date IS NULL, date, id",
    )?;
    let mut names: Vec<String> = Vec::new();
    for name in stmt.query_map([thread_id], |r| r.get::<_, Option<String>>(0))? {
        if let Some(name) = name?
            && !names.contains(&name)
        {
            names.push(name);
            if names.len() == MAX_PARTICIPANTS {
                break;
            }
        }
    }
    Ok(names)
}

const SELECT_SUMMARY: &str =
    "SELECT id, account_id, subject_norm, last_date, message_count, unread_count FROM threads";

pub fn list(
    conn: &Connection,
    account_id: Option<i64>,
    limit: u32,
    offset: u32,
) -> Result<Vec<ThreadSummary>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_SUMMARY}
         WHERE (?1 IS NULL OR account_id = ?1) AND message_count > 0
         ORDER BY last_date IS NULL, last_date DESC, id DESC
         LIMIT ?2 OFFSET ?3"
    ))?;
    let mut threads: Vec<ThreadSummary> = stmt
        .query_map(params![account_id, limit, offset], summary_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    for t in &mut threads {
        t.participants = participants(conn, t.id)?;
    }
    Ok(threads)
}

pub fn count(conn: &Connection, account_id: Option<i64>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM threads WHERE (?1 IS NULL OR account_id = ?1) AND message_count > 0",
        [account_id],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    #[serde(flatten)]
    pub summary: ThreadSummary,
    /// älteste zuerst
    pub messages: Vec<MessageDetail>,
}

pub fn get(conn: &Connection, id: i64) -> Result<ThreadDetail> {
    let mut summary = conn
        .query_row(
            &format!("{SELECT_SUMMARY} WHERE id = ?1"),
            [id],
            summary_from_row,
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Konversation {id}")))?;
    summary.participants = participants(conn, id)?;
    Ok(ThreadDetail {
        summary,
        messages: messages::list_by_thread(conn, id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    struct Seed<'a> {
        message_id: &'a str,
        in_reply_to: Option<&'a str>,
        subject: &'a str,
        date: &'a str,
        is_read: bool,
    }

    fn seed(conn: &Connection, account_id: i64, m: Seed<'_>) -> i64 {
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, name, imap_host, username) VALUES (?1, 'A', 'h', 'u')",
            [account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO imap_folders (id, account_id, path) VALUES (?1, ?1, 'INBOX')",
            [account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, fingerprint, message_id, in_reply_to,
                 from_name, subject, date, is_read, raw_path)
             VALUES (?1, ?1, abs(random() % 1000000), hex(randomblob(8)), ?2, ?3, ?4, ?5, ?6, ?7, 'x.eml')",
            params![
                account_id,
                m.message_id,
                m.in_reply_to,
                format!("Absender {}", m.message_id),
                m.subject,
                format!("{}T10:00:00Z", m.date),
                m.is_read
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn msg<'a>(message_id: &'a str, subject: &'a str, date: &'a str) -> Seed<'a> {
        Seed {
            message_id,
            in_reply_to: None,
            subject,
            date,
            is_read: true,
        }
    }

    fn thread_of(conn: &Connection, message: i64) -> i64 {
        conn.query_row(
            "SELECT thread_id FROM messages WHERE id = ?1",
            [message],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn rebuild_stores_threads_and_counts() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let a = seed(conn, 1, msg("a@x", "Plan", "2026-01-01"));
            let b = seed(
                conn,
                1,
                Seed {
                    in_reply_to: Some("a@x"),
                    is_read: false,
                    ..msg("b@x", "Re: Plan", "2026-01-03")
                },
            );
            let c = seed(conn, 1, msg("c@x", "Anderes", "2026-01-02"));
            assert_eq!(accounts_needing_rebuild(conn)?, vec![1]);

            let stats = rebuild(conn, 1)?;
            assert_eq!((stats.threads, stats.created, stats.removed), (2, 2, 0));
            assert!(accounts_needing_rebuild(conn)?.is_empty());
            assert_eq!(thread_of(conn, a), thread_of(conn, b));
            assert_ne!(thread_of(conn, a), thread_of(conn, c));

            let list = list(conn, Some(1), 50, 0)?;
            assert_eq!(count(conn, Some(1))?, 2);
            let plan = &list[0];
            assert_eq!(plan.subject.as_deref(), Some("Plan"));
            assert_eq!(plan.last_date.as_deref(), Some("2026-01-03T10:00:00Z"));
            assert_eq!((plan.message_count, plan.unread_count), (2, 1));
            assert_eq!(plan.participants, ["Absender a@x", "Absender b@x"]);

            let detail = get(conn, plan.id)?;
            let ids: Vec<i64> = detail.messages.iter().map(|m| m.summary.id).collect();
            assert_eq!(ids, [a, b]);
            assert_eq!(detail.messages[0].summary.thread_id, Some(plan.id));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rebuild_keeps_ids_and_cleans_up() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let a = seed(conn, 1, msg("a@x", "Plan", "2026-01-01"));
            let x = seed(conn, 1, msg("x@x", "Allein", "2026-01-01"));
            rebuild(conn, 1)?;
            let plan = thread_of(conn, a);
            let alone = thread_of(conn, x);

            // Unveränderte Daten: gleiche IDs, nichts neu, nichts entfernt.
            let again = rebuild(conn, 1)?;
            assert_eq!((again.created, again.removed), (0, 0));
            assert_eq!((thread_of(conn, a), thread_of(conn, x)), (plan, alone));

            // Neue Antwort, die beide Konversationen verbindet: die größere
            // bzw. ältere ID bleibt, die andere verschwindet.
            let joint = seed(
                conn,
                1,
                Seed {
                    in_reply_to: Some("a@x"),
                    ..msg("j@x", "Re: Plan", "2026-01-05")
                },
            );
            conn.execute(
                "UPDATE messages SET refs = 'a@x x@x' WHERE id = ?1",
                [joint],
            )?;
            let merged = rebuild(conn, 1)?;
            assert_eq!((merged.threads, merged.created, merged.removed), (1, 0, 1));
            let kept = thread_of(conn, a);
            assert_eq!(kept, plan.min(alone));
            assert_eq!(thread_of(conn, x), kept);
            assert_eq!(thread_of(conn, joint), kept);
            let rows: i64 = conn.query_row("SELECT count(*) FROM threads", [], |r| r.get(0))?;
            assert_eq!(rows, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn accounts_are_threaded_separately() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let one = seed(conn, 1, msg("a@x", "Plan", "2026-01-01"));
            let two = seed(
                conn,
                2,
                Seed {
                    in_reply_to: Some("a@x"),
                    ..msg("b@x", "Re: Plan", "2026-01-02")
                },
            );
            rebuild(conn, 1)?;
            rebuild(conn, 2)?;
            assert_ne!(thread_of(conn, one), thread_of(conn, two));
            assert_eq!(count(conn, Some(1))?, 1);
            assert_eq!(count(conn, None)?, 2);
            Ok(())
        })
        .unwrap();
    }
}
