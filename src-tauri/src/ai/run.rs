//! Ein Vorschlagslauf: die nächsten Mails ohne Vorschlag nacheinander an das
//! Modell schicken und die Antworten als `pending` speichern.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use super::client::{Completion, MessagesClient};
use super::prompt::{self, Choice, MailForPrompt};
use crate::db::Db;
use crate::db::categories;
use crate::db::classifications::{self, Candidate, NewSuggestion};
use crate::error::{Error, Result};

/// Reicht auch, wenn ein Modell trotz `thinking: disabled` kurz nachdenkt.
const MAX_TOKENS: u32 = 2048;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunPhase {
    Running,
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProgress {
    pub phase: RunPhase,
    pub done: usize,
    pub total: usize,
    /// gespeicherte Vorschläge (inklusive nicht auswertbarer)
    pub stored: usize,
    /// Antworten ohne lesbares JSON; als Vorschlag ohne Kategorie gespeichert
    pub unreadable: usize,
    /// Betreff der Mail, die gerade bearbeitet wird
    pub current: Option<String>,
    pub error: Option<String>,
}

fn sender(c: &Candidate) -> Option<String> {
    match (&c.from_name, &c.from_addr) {
        (Some(n), Some(a)) => Some(format!("{n} <{a}>")),
        (Some(x), None) | (None, Some(x)) => Some(x.clone()),
        (None, None) => None,
    }
}

pub async fn suggest_batch(
    db: &Db,
    client: &MessagesClient,
    model: &str,
    limit: u32,
    cancel: &AtomicBool,
    report: &(dyn Fn(RunProgress) + Send + Sync),
) -> Result<RunProgress> {
    let (candidates, cats, tags) = db.with(|c| {
        let cats: Vec<Choice> = categories::list(c)?
            .into_iter()
            .map(|k| Choice {
                id: k.id,
                name: k.name,
                description: k.description,
            })
            .collect();
        let tags: Vec<Choice> = categories::list_tags(c)?
            .into_iter()
            .map(|t| Choice {
                id: t.id,
                name: t.name,
                description: None,
            })
            .collect();
        Ok((classifications::candidates(c, limit)?, cats, tags))
    })?;
    if cats.is_empty() {
        return Err(Error::Invalid(
            "Es gibt noch keine Kategorien. Bitte zuerst unter „Einstellungen“ anlegen.".into(),
        ));
    }
    let system = prompt::system_prompt(&cats, &tags);

    let mut progress = RunProgress {
        phase: RunPhase::Running,
        done: 0,
        total: candidates.len(),
        stored: 0,
        unreadable: 0,
        current: None,
        error: None,
    };
    report(progress.clone());

    for mail in &candidates {
        if cancel.load(Ordering::Relaxed) {
            progress.phase = RunPhase::Cancelled;
            break;
        }
        progress.current = Some(
            mail.subject
                .clone()
                .unwrap_or_else(|| "(kein Betreff)".into()),
        );
        report(progress.clone());

        let from = sender(mail);
        let user = prompt::user_prompt(&MailForPrompt {
            from: from.as_deref(),
            subject: mail.subject.as_deref(),
            date: mail.date.as_deref(),
            body: mail.body_text.as_deref(),
        });
        let answer = client
            .complete(&Completion {
                model,
                system: &system,
                user: &user,
                max_tokens: MAX_TOKENS,
            })
            .await;
        let answer = match answer {
            Ok(a) => a,
            // Server- und Verbindungsfehler betreffen alle weiteren Mails auch.
            Err(e) => {
                progress.phase = RunPhase::Failed;
                progress.error = Some(e.to_string());
                progress.current = None;
                report(progress.clone());
                return Err(e);
            }
        };

        let suggestion = match prompt::parse(&answer, &cats, &tags) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Mail {}: {e}", mail.message_id);
                progress.unreadable += 1;
                prompt::Suggestion {
                    reason: Some("Die Antwort des Modells war nicht auswertbar.".into()),
                    ..Default::default()
                }
            }
        };
        db.with(|c| {
            classifications::store_suggestion(
                c,
                &NewSuggestion {
                    message_id: mail.message_id,
                    category_id: suggestion.category_id,
                    confidence: suggestion.confidence,
                    reason: suggestion.reason.as_deref(),
                    model,
                    tag_ids: &suggestion.tag_ids,
                },
            )
        })?;
        progress.stored += 1;
        progress.done += 1;
    }

    if progress.phase == RunPhase::Running {
        progress.phase = RunPhase::Done;
    }
    progress.current = None;
    report(progress.clone());
    Ok(progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Antwortet auf jede Anfrage mit derselben Modellantwort.
    async fn fake_model(answer: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 65536];
                let mut read = 0;
                // Kopf und Body lesen (Body ist klein, Länge aus dem Kopf).
                loop {
                    let n = sock.read(&mut buf[read..]).await.unwrap();
                    read += n;
                    let text = String::from_utf8_lossy(&buf[..read]);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len: usize = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if read >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let body =
                    serde_json::json!({"content": [{"type": "text", "text": answer}]}).to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        url
    }

    fn seed(db: &Db, count: i64) {
        db.with(|c| {
            c.execute_batch(
                "INSERT INTO accounts (id, name, imap_host, username) VALUES (1, 'A', 'h', 'u');
                 INSERT INTO imap_folders (id, account_id, path) VALUES (1, 1, 'INBOX');",
            )?;
            for i in 1..=count {
                c.execute(
                    "INSERT INTO messages (id, account_id, folder_id, uid, fingerprint, raw_path, subject, date, body_text)
                     VALUES (?1, 1, 1, ?1, ?1, 'x', 'Flug', ?2, 'Ihre Buchung')",
                    rusqlite::params![i, format!("2026-01-{i:02}T00:00:00Z")],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn stores_pending_suggestions_up_to_limit() {
        let db = Db::open_in_memory().unwrap();
        seed(&db, 3);
        let url = fake_model(
            r#"{"category":"Reisen","confidence":0.9,"reason":"Flugbuchung","tags":[]}"#,
        )
        .await;
        let client = MessagesClient::new(&url, None).unwrap();
        let seen = Mutex::new(Vec::new());
        let report = |p: RunProgress| seen.lock().unwrap().push(p.phase);

        let result = suggest_batch(
            &db,
            &client,
            "test-model",
            2,
            &AtomicBool::new(false),
            &report,
        )
        .await
        .unwrap();
        assert_eq!(
            (result.phase, result.done, result.stored, result.unreadable),
            (RunPhase::Done, 2, 2, 0)
        );
        assert_eq!(seen.lock().unwrap().last(), Some(&RunPhase::Done));

        db.with(|c| {
            let items = classifications::pending(c, 10, 0)?;
            assert_eq!(items.len(), 2);
            assert!(
                items
                    .iter()
                    .all(|i| i.category_name.as_deref() == Some("Reisen"))
            );
            assert_eq!(items[0].model.as_deref(), Some("test-model"));
            // die neuesten zuerst: Mail 3 und 2, Mail 1 bleibt übrig
            assert_eq!(classifications::count_candidates(c)?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn unreadable_answers_are_stored_without_category() {
        let db = Db::open_in_memory().unwrap();
        seed(&db, 1);
        let url = fake_model("Das ist wohl eine Reise.").await;
        let client = MessagesClient::new(&url, None).unwrap();
        let result = suggest_batch(&db, &client, "m", 50, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap();
        assert_eq!((result.stored, result.unreadable), (1, 1));
        db.with(|c| {
            let item = &classifications::pending(c, 10, 0)?[0];
            assert_eq!(item.category_id, None);
            assert!(item.reason.as_deref().unwrap().contains("nicht auswertbar"));
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn cancel_and_server_errors_stop_the_run() {
        let db = Db::open_in_memory().unwrap();
        seed(&db, 2);
        let url = fake_model("{}").await;
        let client = MessagesClient::new(&url, None).unwrap();
        let cancelled = suggest_batch(&db, &client, "m", 50, &AtomicBool::new(true), &|_| {})
            .await
            .unwrap();
        assert_eq!((cancelled.phase, cancelled.done), (RunPhase::Cancelled, 0));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let client = MessagesClient::new(&dead, None).unwrap();
        let err = suggest_batch(&db, &client, "m", 50, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nicht erreichbar"));
        assert_eq!(db.with(|c| classifications::count_pending(c)).unwrap(), 0);
    }
}
