//! KI-Vorschläge und ihre Prüfung.
//!
//! Regel 4: Ein Vorschlag wird immer mit `status = 'pending'` gespeichert und
//! ändert erst durch [`decide`] (also durch dich im Review) seinen Status.

use std::collections::HashSet;

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::messages::{self, MessageDetail};
use crate::error::{Error, Result};

/// Eine Mail, für die noch kein Vorschlag existiert.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub message_id: i64,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub body_text: Option<String>,
}

const NO_CLASSIFICATION: &str =
    "NOT EXISTS (SELECT 1 FROM classifications k WHERE k.message_id = m.id)";

/// Mails ohne jeden Vorschlag (auch ohne abgelehnten), die neuesten zuerst.
pub fn candidates(conn: &Connection, limit: u32) -> Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT m.id, m.from_name, m.from_addr, m.subject, m.date, m.body_text
         FROM messages m WHERE {NO_CLASSIFICATION}
         ORDER BY m.date IS NULL, m.date DESC, m.id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], |r| {
        Ok(Candidate {
            message_id: r.get(0)?,
            from_name: r.get(1)?,
            from_addr: r.get(2)?,
            subject: r.get(3)?,
            date: r.get(4)?,
            body_text: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn count_candidates(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        &format!("SELECT count(*) FROM messages m WHERE {NO_CLASSIFICATION}"),
        [],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone)]
pub struct NewSuggestion<'a> {
    pub message_id: i64,
    /// `None`: keine der Kategorien passt (oder Antwort nicht auswertbar)
    pub category_id: Option<i64>,
    pub confidence: Option<f64>,
    pub reason: Option<&'a str>,
    pub model: &'a str,
    pub tag_ids: &'a [i64],
}

/// Speichert einen KI-Vorschlag als `pending`, samt vorgeschlagener Tags.
pub fn store_suggestion(conn: &mut Connection, s: &NewSuggestion<'_>) -> Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO classifications
             (message_id, category_id, suggested_category_id, source, confidence, reason, status, model)
         VALUES (?1, ?2, ?2, 'ai', ?3, ?4, 'pending', ?5)",
        params![s.message_id, s.category_id, s.confidence, s.reason, s.model],
    )?;
    let id = tx.last_insert_rowid();
    for tag_id in s.tag_ids {
        tx.execute(
            "INSERT OR IGNORE INTO message_tags (message_id, tag_id, source, status)
             VALUES (?1, ?2, 'ai', 'pending')",
            params![s.message_id, tag_id],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTag {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub id: i64,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub confidence: Option<f64>,
    pub reason: Option<String>,
    pub model: Option<String>,
    pub tags: Vec<ReviewTag>,
    pub message: MessageDetail,
}

fn tags_of(conn: &Connection, message_id: i64) -> Result<Vec<ReviewTag>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.id, t.name, mt.source, mt.status FROM message_tags mt
         JOIN tags t ON t.id = mt.tag_id WHERE mt.message_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([message_id], |r| {
        Ok(ReviewTag {
            id: r.get(0)?,
            name: r.get(1)?,
            source: r.get(2)?,
            status: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Offene Vorschläge, in der Reihenfolge der Mails (neueste zuerst).
pub fn pending(conn: &Connection, limit: u32, offset: u32) -> Result<Vec<ReviewItem>> {
    let mut stmt = conn.prepare(
        "SELECT k.id, k.message_id, k.category_id, c.name, k.confidence, k.reason, k.model
         FROM classifications k
         JOIN messages m ON m.id = k.message_id
         LEFT JOIN categories c ON c.id = k.category_id
         WHERE k.status = 'pending'
         ORDER BY m.date IS NULL, m.date DESC, k.id DESC
         LIMIT ?1 OFFSET ?2",
    )?;
    // Erst die Zeilen, dann je Mail Details und Tags (eigene Abfragen).
    let rows: Vec<(i64, ReviewItemHead)> = stmt
        .query_map(params![limit, offset], |r| {
            Ok((
                r.get(1)?,
                ReviewItemHead {
                    id: r.get(0)?,
                    category_id: r.get(2)?,
                    category_name: r.get(3)?,
                    confidence: r.get(4)?,
                    reason: r.get(5)?,
                    model: r.get(6)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;

    rows.into_iter()
        .map(|(message_id, h)| {
            Ok(ReviewItem {
                id: h.id,
                category_id: h.category_id,
                category_name: h.category_name,
                confidence: h.confidence,
                reason: h.reason,
                model: h.model,
                tags: tags_of(conn, message_id)?,
                message: messages::get(conn, message_id)?,
            })
        })
        .collect()
}

struct ReviewItemHead {
    id: i64,
    category_id: Option<i64>,
    category_name: Option<String>,
    confidence: Option<f64>,
    reason: Option<String>,
    model: Option<String>,
}

pub fn count_pending(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM classifications WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Decision {
    /// Vorschlag übernehmen
    Accept,
    /// andere Kategorie wählen
    #[serde(rename_all = "camelCase")]
    Change { category_id: i64 },
    /// Vorschlag verwerfen, die Mail bleibt ohne Kategorie
    Reject,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInput {
    pub decision: Decision,
    /// Tags, die die Mail behalten bzw. bekommen soll (bei Ablehnen ignoriert)
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}

/// Übernimmt, ändert oder verwirft einen offenen Vorschlag.
pub fn decide(conn: &mut Connection, id: i64, input: &DecisionInput) -> Result<()> {
    let tx = conn.transaction()?;
    let (message_id, status, suggested): (i64, String, Option<i64>) = tx
        .query_row(
            "SELECT message_id, status, suggested_category_id FROM classifications WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("Vorschlag {id}")))?;
    if status != "pending" {
        return Err(Error::Invalid(
            "Dieser Vorschlag ist bereits entschieden".into(),
        ));
    }

    let (new_status, category_id) = match input.decision {
        Decision::Accept => match suggested {
            Some(c) => ("accepted", Some(c)),
            None => {
                return Err(Error::Invalid(
                    "Ohne vorgeschlagene Kategorie bitte eine Kategorie wählen oder ablehnen"
                        .into(),
                ));
            }
        },
        Decision::Change { category_id } => {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM categories WHERE id = ?1)",
                [category_id],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(Error::NotFound(format!("Kategorie {category_id}")));
            }
            let status = if suggested == Some(category_id) {
                "accepted"
            } else {
                "changed"
            };
            (status, Some(category_id))
        }
        Decision::Reject => ("rejected", suggested),
    };

    tx.execute(
        "UPDATE classifications
         SET status = ?2, category_id = ?3, decided_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, new_status, category_id],
    )
    .map_err(|e| match e.sqlite_error_code() {
        Some(ErrorCode::ConstraintViolation) => {
            Error::Invalid("Die Mail hat bereits eine bestätigte Kategorie".into())
        }
        _ => e.into(),
    })?;

    let keep: HashSet<i64> = if input.decision == Decision::Reject {
        HashSet::new()
    } else {
        input.tag_ids.iter().copied().collect()
    };
    let suggested_tags: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT tag_id FROM message_tags WHERE message_id = ?1 AND status = 'pending'",
        )?;
        stmt.query_map([message_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for tag_id in &suggested_tags {
        let status = if keep.contains(tag_id) {
            "accepted"
        } else {
            "rejected"
        };
        tx.execute(
            "UPDATE message_tags SET status = ?3 WHERE message_id = ?1 AND tag_id = ?2",
            params![message_id, tag_id, status],
        )?;
    }
    for tag_id in keep.iter().filter(|t| !suggested_tags.contains(t)) {
        tx.execute(
            "INSERT INTO message_tags (message_id, tag_id, source, status)
             VALUES (?1, ?2, 'user', 'accepted')
             ON CONFLICT (message_id, tag_id) DO UPDATE SET status = 'accepted'",
            params![message_id, tag_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, categories};

    fn setup(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u');
             INSERT INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');
             INSERT INTO messages (id, account_id, folder_id, uid, fingerprint, raw_path, subject, date)
               VALUES (1, 1, 1, 1, 'a', 'x', 'Alt', '2026-01-01T00:00:00Z'),
                      (2, 1, 1, 2, 'b', 'x', 'Neu', '2026-02-01T00:00:00Z'),
                      (3, 1, 1, 3, 'c', 'x', 'Ohne Datum', NULL);
             INSERT INTO tags (id, name) VALUES (1, 'wichtig'), (2, 'steuer'), (3, 'privat');",
        )
        .unwrap();
    }

    fn category(conn: &Connection, name: &str) -> i64 {
        categories::list(conn)
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap()
            .id
    }

    fn suggest(
        conn: &mut Connection,
        message_id: i64,
        category_id: Option<i64>,
        tags: &[i64],
    ) -> i64 {
        store_suggestion(
            conn,
            &NewSuggestion {
                message_id,
                category_id,
                confidence: Some(0.8),
                reason: Some("passt"),
                model: "test",
                tag_ids: tags,
            },
        )
        .unwrap()
    }

    fn row(conn: &Connection, id: i64) -> (String, Option<i64>, Option<i64>) {
        conn.query_row(
            "SELECT status, category_id, suggested_category_id FROM classifications WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn tag_status(conn: &Connection, message_id: i64) -> Vec<(i64, String, String)> {
        let mut stmt = conn
            .prepare("SELECT tag_id, source, status FROM message_tags WHERE message_id = ?1 ORDER BY tag_id")
            .unwrap();
        stmt.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn candidates_skip_mails_with_any_suggestion() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            setup(c);
            let ids: Vec<i64> = candidates(c, 10)?.iter().map(|m| m.message_id).collect();
            assert_eq!(ids, [2, 1, 3], "neueste zuerst, ohne Datum zuletzt");
            assert_eq!(candidates(c, 1)?.len(), 1);

            let reisen = category(c, "Reisen");
            let id = suggest(c, 2, Some(reisen), &[]);
            decide(
                c,
                id,
                &DecisionInput {
                    decision: Decision::Reject,
                    tag_ids: vec![],
                },
            )?;
            suggest(c, 1, None, &[]);
            assert_eq!(count_candidates(c)?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn suggestions_are_pending_until_decided() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            setup(c);
            let reisen = category(c, "Reisen");
            let id = suggest(c, 2, Some(reisen), &[1, 2]);
            assert_eq!(row(c, id), ("pending".into(), Some(reisen), Some(reisen)));
            assert_eq!(count_pending(c)?, 1);

            let items = pending(c, 10, 0)?;
            assert_eq!(items[0].category_name.as_deref(), Some("Reisen"));
            assert_eq!(items[0].message.summary.subject.as_deref(), Some("Neu"));
            assert_eq!(
                items[0].message.summary.category, None,
                "noch nicht bestätigt"
            );
            let tag_names: Vec<_> = items[0].tags.iter().map(|t| t.name.as_str()).collect();
            assert_eq!(tag_names, ["steuer", "wichtig"]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn accept_keeps_suggestion_and_selected_tags() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            setup(c);
            let reisen = category(c, "Reisen");
            let id = suggest(c, 2, Some(reisen), &[1, 2]);
            decide(
                c,
                id,
                &DecisionInput {
                    decision: Decision::Accept,
                    tag_ids: vec![1, 3],
                },
            )?;

            assert_eq!(row(c, id), ("accepted".into(), Some(reisen), Some(reisen)));
            assert_eq!(
                tag_status(c, 2),
                [
                    (1, "ai".into(), "accepted".into()),
                    (2, "ai".into(), "rejected".into()),
                    (3, "user".into(), "accepted".into()),
                ]
            );
            assert_eq!(
                messages::get(c, 2)?.summary.category.as_deref(),
                Some("Reisen")
            );
            assert_eq!(count_pending(c)?, 0);
            assert!(matches!(
                decide(
                    c,
                    id,
                    &DecisionInput {
                        decision: Decision::Reject,
                        tag_ids: vec![]
                    }
                ),
                Err(Error::Invalid(_))
            ));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn change_keeps_original_suggestion() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            setup(c);
            let (reisen, finanzen) = (category(c, "Reisen"), category(c, "Finanzen"));
            let id = suggest(c, 2, Some(reisen), &[]);
            let change = Decision::Change {
                category_id: finanzen,
            };
            decide(
                c,
                id,
                &DecisionInput {
                    decision: change,
                    tag_ids: vec![],
                },
            )?;
            assert_eq!(row(c, id), ("changed".into(), Some(finanzen), Some(reisen)));

            // „Ändern“ auf die vorgeschlagene Kategorie zählt als Übernehmen.
            let same = suggest(c, 1, Some(reisen), &[]);
            let change = Decision::Change {
                category_id: reisen,
            };
            decide(
                c,
                same,
                &DecisionInput {
                    decision: change,
                    tag_ids: vec![],
                },
            )?;
            assert_eq!(row(c, same).0, "accepted");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reject_and_missing_category() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            setup(c);
            let none = suggest(c, 3, None, &[1]);
            assert!(matches!(
                decide(
                    c,
                    none,
                    &DecisionInput {
                        decision: Decision::Accept,
                        tag_ids: vec![]
                    }
                ),
                Err(Error::Invalid(_))
            ));
            decide(
                c,
                none,
                &DecisionInput {
                    decision: Decision::Reject,
                    tag_ids: vec![1],
                },
            )?;
            assert_eq!(row(c, none).0, "rejected");
            assert_eq!(tag_status(c, 3), [(1, "ai".into(), "rejected".into())]);
            assert_eq!(messages::get(c, 3)?.summary.category, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn decision_json_shape() {
        let accept: DecisionInput =
            serde_json::from_str(r#"{"decision":{"kind":"accept"},"tagIds":[1]}"#).unwrap();
        assert_eq!(accept.decision, Decision::Accept);
        let change: DecisionInput =
            serde_json::from_str(r#"{"decision":{"kind":"change","categoryId":4}}"#).unwrap();
        assert_eq!(change.decision, Decision::Change { category_id: 4 });
        assert!(change.tag_ids.is_empty());
    }
}
