//! Antworten, Weiterleitungen und ausgehende Nachrichten aufbauen.
//!
//! [`prepare`] macht aus einer archivierten Mail die Vorbelegung eines
//! Entwurfs (Empfänger, Betreff, Zitat, Anhänge). [`build`] macht aus einem
//! Entwurf die fertige MIME-Nachricht für SMTP und die lokale Ablage.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart};
use mail_parser::MessageParser;
use sha2::{Digest, Sha256};

use super::body;
use super::html;
use super::parse::normalize_message_id;
use super::thread::normalize_subject;
use crate::db::drafts::{DraftAttachment, DraftKind, Recipient};
use crate::db::identities::Identity;
use crate::error::{Error, Result};

/// Vorbelegung eines Entwurfs aus einer archivierten Mail.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prepared {
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub subject: String,
    /// Leerzeile zum Schreiben, Signatur, Zitat
    pub body_html: String,
    pub attachments: Vec<DraftAttachment>,
    /// Welche eigene Adresse angeschrieben war (für die Identitätswahl)
    pub addressed_identity: Option<String>,
}

fn recipients(list: Option<mail_parser::AddressList<'_>>) -> Vec<Recipient> {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| {
                let address = m.address()?.trim().to_lowercase();
                (!address.is_empty()).then(|| Recipient {
                    name: m
                        .name()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(String::from),
                    address,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

fn is_own(address: &str, own: &[String]) -> bool {
    own.iter().any(|o| o.eq_ignore_ascii_case(address))
}

fn dedupe(list: Vec<Recipient>, exclude: &[String], also_exclude: &[Recipient]) -> Vec<Recipient> {
    let mut seen: Vec<String> = also_exclude.iter().map(|r| r.address.clone()).collect();
    list.into_iter()
        .filter(|r| !is_own(&r.address, exclude))
        .filter(|r| {
            if seen.iter().any(|s| s.eq_ignore_ascii_case(&r.address)) {
                false
            } else {
                seen.push(r.address.clone());
                true
            }
        })
        .collect()
}

fn display(r: &Recipient) -> String {
    match &r.name {
        Some(n) => format!("{n} <{}>", r.address),
        None => r.address.clone(),
    }
}

fn german_date(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|d| d.format("%d.%m.%Y um %H:%M UTC").to_string())
        .unwrap_or_else(|| "unbekanntem Datum".into())
}

/// Signaturblock: Trenner `-- ` wie in RFC 3676.
pub fn signature_block(identity: Option<&Identity>) -> String {
    let Some(id) = identity else {
        return String::new();
    };
    match (&id.signature_html, &id.signature_text) {
        (Some(h), _) => format!("<div class=\"signature\">-- <br>{h}</div>"),
        (None, Some(t)) => format!(
            "<div class=\"signature\">-- <br>{}</div>",
            html::text_to_fragment(t)
        ),
        (None, None) => String::new(),
    }
}

/// Körper der Originalmail als bereinigtes HTML-Fragment (ohne externe Bilder).
fn original_fragment(msg: &mail_parser::Message<'_>) -> Result<String> {
    use mail_parser::PartKind;
    let has_html = msg.html_body().any(|p| matches!(p.kind(), PartKind::Html));
    if has_html {
        let parts: Vec<String> = msg.html_bodies().map(|c| c.into_owned()).collect();
        let (frag, _) = html::sanitize_fragment(&parts.join("<hr>"), &HashMap::new(), false)?;
        Ok(frag)
    } else {
        let text: Vec<String> = msg.text_bodies().map(|c| c.into_owned()).collect();
        Ok(html::text_to_fragment(&text.join("\n\n")))
    }
}

pub fn prepare(
    raw: &[u8],
    message_db_id: i64,
    kind: DraftKind,
    own_addresses: &[String],
    identity: Option<&Identity>,
) -> Result<Prepared> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Parse("Mail-Datei nicht lesbar".into()))?;

    let from = recipients(msg.from());
    let reply_to = recipients(msg.reply_to());
    let orig_to = recipients(msg.to());
    let orig_cc = recipients(msg.cc());
    let addressed_identity = orig_to
        .iter()
        .chain(orig_cc.iter())
        .find(|r| is_own(&r.address, own_addresses))
        .map(|r| r.address.clone());

    let subject_norm = normalize_subject(msg.subject().unwrap_or("")).display;
    let (to, cc, subject, attachments) = match kind {
        DraftKind::Reply | DraftKind::ReplyAll => {
            let primary = if reply_to.is_empty() {
                from.clone()
            } else {
                reply_to.clone()
            };
            // Antwort auf die eigene Mail: an die ursprünglichen Empfänger.
            let primary = if primary.iter().all(|r| is_own(&r.address, own_addresses)) {
                orig_to.clone()
            } else {
                primary
            };
            let to = dedupe(primary, own_addresses, &[]);
            let cc = if kind == DraftKind::ReplyAll {
                let rest: Vec<Recipient> = orig_to.iter().chain(orig_cc.iter()).cloned().collect();
                dedupe(rest, own_addresses, &to)
            } else {
                Vec::new()
            };
            let subject = if subject_norm.is_empty() {
                "Re:".to_string()
            } else {
                format!("Re: {subject_norm}")
            };
            (to, cc, subject, Vec::new())
        }
        DraftKind::Forward => {
            let attachments = body::attachments(raw)?
                .into_iter()
                .map(|a| DraftAttachment::Message {
                    message_id: message_db_id,
                    part: a.part,
                    name: a.filename,
                    mime: a.mime_type,
                    size: a.size,
                })
                .collect();
            let subject = if subject_norm.is_empty() {
                "Fwd:".to_string()
            } else {
                format!("Fwd: {subject_norm}")
            };
            (Vec::new(), Vec::new(), subject, attachments)
        }
        DraftKind::New => (Vec::new(), Vec::new(), String::new(), Vec::new()),
    };

    let original = original_fragment(&msg)?;
    let sender = from
        .first()
        .map(display)
        .unwrap_or_else(|| "unbekannt".into());
    let date = msg.date().map(|d| german_date(d.to_timestamp()));
    let quoted = match kind {
        DraftKind::Reply | DraftKind::ReplyAll => format!(
            "<div class=\"quote-header\">Am {} schrieb {}:</div>\
             <blockquote type=\"cite\" style=\"margin:0 0 0 8px;padding-left:12px;border-left:2px solid #ccc\">{original}</blockquote>",
            date.as_deref().unwrap_or("unbekanntem Datum"),
            html::escape(&sender)
        ),
        DraftKind::Forward => {
            let mut head = String::from(
                "<div class=\"forward-header\">-------- Weitergeleitete Nachricht --------<br>",
            );
            head.push_str(&format!("Von: {}<br>", html::escape(&sender)));
            if let Some(d) = &date {
                head.push_str(&format!("Datum: {}<br>", html::escape(d)));
            }
            head.push_str(&format!(
                "Betreff: {}<br>",
                html::escape(msg.subject().unwrap_or(""))
            ));
            let to_line = orig_to.iter().map(display).collect::<Vec<_>>().join(", ");
            if !to_line.is_empty() {
                head.push_str(&format!("An: {}<br>", html::escape(&to_line)));
            }
            head.push_str("</div><br>");
            format!("{head}{original}")
        }
        DraftKind::New => String::new(),
    };

    let body_html = format!("<div><br></div>{}{}", signature_block(identity), quoted);

    Ok(Prepared {
        to,
        cc,
        subject,
        body_html,
        attachments,
        addressed_identity,
    })
}

/// HTML zu lesbarem Klartext für den `text/plain`-Teil.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    let bytes = html.as_bytes();
    let mut quote_depth = 0usize;
    let mut at_line_start = true;
    let mut skip_until: Option<&str> = None;
    let lower = html.to_ascii_lowercase();

    fn push_newline(out: &mut String, depth: usize, at_start: &mut bool) {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("> ");
        }
        *at_start = true;
    }

    while i < bytes.len() {
        if let Some(end_tag) = skip_until {
            match lower[i..].find(end_tag) {
                Some(pos) => {
                    i += pos + end_tag.len();
                    skip_until = None;
                }
                None => break,
            }
            continue;
        }
        if bytes[i] == b'<' {
            let Some(close) = html[i..].find('>') else {
                break;
            };
            let tag = &lower[i + 1..i + close];
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let closing = tag.starts_with('/');
            match name.as_str() {
                "script" | "style" | "head" | "title" if !closing => {
                    skip_until = Some(match name.as_str() {
                        "script" => "</script>",
                        "style" => "</style>",
                        "head" => "</head>",
                        _ => "</title>",
                    });
                }
                "br" => push_newline(&mut out, quote_depth, &mut at_line_start),
                "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
                | "table" | "pre" => {
                    if !at_line_start {
                        push_newline(&mut out, quote_depth, &mut at_line_start);
                    }
                    if !closing
                        && matches!(name.as_str(), "p" | "h1" | "h2" | "h3")
                        && !out.is_empty()
                    {
                        push_newline(&mut out, quote_depth, &mut at_line_start);
                    }
                }
                "li" if !closing => {
                    if !at_line_start {
                        push_newline(&mut out, quote_depth, &mut at_line_start);
                    }
                    out.push_str("- ");
                    at_line_start = false;
                }
                "blockquote" => {
                    if !at_line_start {
                        push_newline(&mut out, quote_depth, &mut at_line_start);
                    }
                    if closing {
                        quote_depth = quote_depth.saturating_sub(1);
                    } else {
                        quote_depth += 1;
                    }
                    // Präfix der neuen Zeile an die Tiefe anpassen
                    while out.ends_with("> ") {
                        out.truncate(out.len() - 2);
                    }
                    for _ in 0..quote_depth {
                        out.push_str("> ");
                    }
                }
                _ => {}
            }
            i += close + 1;
            continue;
        }
        if bytes[i] == b'&'
            && let Some(semi) = html[i..].find(';').filter(|&s| s < 10)
        {
            let entity = &html[i + 1..i + semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" | "#160" => Some(' '),
                e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = decoded {
                out.push(c);
                at_line_start = false;
                i += semi + 1;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        if ch.is_whitespace() {
            if !at_line_start && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
            at_line_start = false;
        }
        i += ch.len_utf8();
    }
    // Zeilen von Leerraum am Ende befreien, mehr als zwei Leerzeilen kappen.
    let mut lines: Vec<String> = out.lines().map(|l| l.trim_end().to_string()).collect();
    lines.dedup_by(|a, b| a.trim().is_empty() && b.trim().is_empty());
    lines.join("\n").trim().to_string()
}

/// Eindeutige Message-ID für ausgehende Mails: `<zeit.hash@domain>`.
pub fn new_message_id(from: &str, subject: &str) -> String {
    let now = Utc::now();
    let domain = from.rsplit('@').next().unwrap_or("localhost");
    let mut h = Sha256::new();
    h.update(now.timestamp_nanos_opt().unwrap_or_default().to_le_bytes());
    h.update(from.as_bytes());
    h.update(subject.as_bytes());
    let digest = h.finalize();
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("<{}.{hex}@{domain}>", now.format("%Y%m%d%H%M%S"))
}

/// Antwort-Header aus der Originalmail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplyHeaders {
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

impl ReplyHeaders {
    /// `message_id` und `refs` der Originalmail, wie im Archiv gespeichert.
    pub fn from_original(message_id: Option<&str>, refs: Option<&str>) -> Self {
        let id = message_id.and_then(normalize_message_id);
        let references = match (&id, refs) {
            (Some(id), Some(r)) if !r.trim().is_empty() => Some(format!("{} {id}", r.trim())),
            (Some(id), _) => Some(id.clone()),
            (None, Some(r)) if !r.trim().is_empty() => Some(r.trim().to_string()),
            _ => None,
        };
        Self {
            in_reply_to: id,
            references,
        }
    }
}

/// Fertige Anhangsdaten fürs Senden.
pub struct OutgoingAttachment {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub struct Outgoing<'a> {
    pub identity: &'a Identity,
    pub to: &'a [Recipient],
    pub cc: &'a [Recipient],
    pub bcc: &'a [Recipient],
    pub subject: &'a str,
    /// HTML aus dem Editor; wird vor dem Senden bereinigt
    pub body_html: &'a str,
    pub reply: &'a ReplyHeaders,
    pub attachments: Vec<OutgoingAttachment>,
}

fn mailbox(r: &Recipient) -> Result<Mailbox> {
    let s = match &r.name {
        Some(n) => format!("{} <{}>", n.replace(['<', '>', '"'], ""), r.address),
        None => r.address.clone(),
    };
    s.parse::<Mailbox>()
        .map_err(|e| Error::Invalid(format!("Adresse „{}“: {e}", r.address)))
}

/// Baut die MIME-Nachricht. Liefert sie zusammen mit ihrer Message-ID.
pub fn build(o: &Outgoing<'_>) -> Result<(lettre::Message, String)> {
    if o.to.is_empty() && o.cc.is_empty() && o.bcc.is_empty() {
        return Err(Error::Invalid("Mindestens ein Empfänger ist nötig".into()));
    }
    let from = mailbox(&Recipient {
        name: Some(o.identity.name.clone()),
        address: o.identity.email.clone(),
    })?;
    let message_id = new_message_id(&o.identity.email, o.subject);

    let mut b = lettre::Message::builder()
        .from(from)
        .subject(o.subject)
        .message_id(Some(message_id.clone()))
        .date_now()
        .user_agent("Mail-Assistent".into());
    for r in o.to {
        b = b.to(mailbox(r)?);
    }
    for r in o.cc {
        b = b.cc(mailbox(r)?);
    }
    for r in o.bcc {
        b = b.bcc(mailbox(r)?);
    }
    if let Some(id) = &o.reply.in_reply_to {
        b = b.in_reply_to(format!("<{id}>"));
    }
    if let Some(refs) = &o.reply.references {
        let formatted = refs
            .split_whitespace()
            .map(|r| format!("<{r}>"))
            .collect::<Vec<_>>()
            .join(" ");
        b = b.references(formatted);
    }

    let (clean, _) = html::sanitize_fragment(o.body_html, &HashMap::new(), true)?;
    let html_doc = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head>\
         <body style=\"font:14px/1.5 -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif\">{clean}</body></html>"
    );
    let text = html_to_text(&clean);
    let alternative = MultiPart::alternative_plain_html(text, html_doc);

    let message = if o.attachments.is_empty() {
        b.multipart(alternative)
    } else {
        let mut mixed = MultiPart::mixed().multipart(alternative);
        for a in &o.attachments {
            let ct = ContentType::parse(&a.mime)
                .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            mixed = mixed.singlepart(Attachment::new(a.name.clone()).body(a.bytes.clone(), ct));
        }
        b.multipart(mixed)
    }
    .map_err(|e| Error::Other(format!("Nachricht konnte nicht gebaut werden: {e}")))?;

    Ok((message, message_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = "From: Anna Beispiel <anna@example.org>\r\nTo: Ich <me@mail.de>, Tom <tom@example.org>\r\nCc: cc@example.org\r\nReply-To: antworten@example.org\r\nSubject: AW: Re: Sommerfest\r\nDate: Mon, 27 Oct 2026 09:15:00 +0000\r\nMessage-ID: <orig@example.org>\r\nReferences: <root@example.org>\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Wer bringt was <b>mit</b>?<script>x()</script></p>";

    fn identity() -> Identity {
        Identity {
            id: 1,
            account_id: 1,
            name: "Adrian".into(),
            email: "me@mail.de".into(),
            signature_text: Some("Viele Grüße\nAdrian".into()),
            signature_html: None,
            is_default: true,
        }
    }

    fn own() -> Vec<String> {
        vec!["me@mail.de".into()]
    }

    #[test]
    fn reply_uses_reply_to_and_strips_prefixes() {
        let p = prepare(
            ORIGINAL.as_bytes(),
            7,
            DraftKind::Reply,
            &own(),
            Some(&identity()),
        )
        .unwrap();
        assert_eq!(p.to.len(), 1);
        assert_eq!(p.to[0].address, "antworten@example.org");
        assert!(p.cc.is_empty());
        assert_eq!(p.subject, "Re: Sommerfest");
        assert_eq!(p.addressed_identity.as_deref(), Some("me@mail.de"));
        assert!(
            p.body_html
                .starts_with("<div><br></div><div class=\"signature\">-- <br>")
        );
        assert!(p.body_html.contains("Viele Grüße<br>Adrian"));
        assert!(p.body_html.contains(
            "Am 27.10.2026 um 09:15 UTC schrieb Anna Beispiel &lt;anna@example.org&gt;:"
        ));
        assert!(p.body_html.contains("<blockquote"));
        assert!(p.body_html.contains("Wer bringt was <b>mit</b>?"));
        assert!(!p.body_html.contains("<script"));
        assert!(p.attachments.is_empty());
    }

    #[test]
    fn reply_all_excludes_own_and_duplicates() {
        let p = prepare(ORIGINAL.as_bytes(), 7, DraftKind::ReplyAll, &own(), None).unwrap();
        let to: Vec<_> = p.to.iter().map(|r| r.address.as_str()).collect();
        let cc: Vec<_> = p.cc.iter().map(|r| r.address.as_str()).collect();
        assert_eq!(to, ["antworten@example.org"]);
        assert_eq!(cc, ["tom@example.org", "cc@example.org"]);
        assert!(!p.body_html.contains("signature"));
    }

    #[test]
    fn reply_to_own_mail_goes_to_original_recipients() {
        let raw = "From: Ich <me@mail.de>\r\nTo: tom@example.org\r\nSubject: Hallo\r\n\r\nText";
        let p = prepare(raw.as_bytes(), 1, DraftKind::Reply, &own(), None).unwrap();
        assert_eq!(p.to[0].address, "tom@example.org");
        assert_eq!(p.subject, "Re: Hallo");
    }

    #[test]
    fn forward_carries_attachments_and_header() {
        let raw = "From: a@x.de\r\nTo: me@mail.de\r\nSubject: Rechnung\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nAnbei.\r\n--b\r\nContent-Type: application/pdf; name=r.pdf\r\nContent-Disposition: attachment; filename=r.pdf\r\n\r\n%PDF\r\n--b--\r\n";
        let p = prepare(raw.as_bytes(), 42, DraftKind::Forward, &own(), None).unwrap();
        assert!(p.to.is_empty());
        assert_eq!(p.subject, "Fwd: Rechnung");
        assert!(p.body_html.contains("Weitergeleitete Nachricht"));
        assert!(p.body_html.contains("Von: a@x.de"));
        assert!(p.body_html.contains("Anbei."));
        assert!(
            matches!(&p.attachments[0], DraftAttachment::Message { message_id: 42, name, .. } if name == "r.pdf")
        );
    }

    #[test]
    fn html_to_text_handles_structure_and_quotes() {
        let t = html_to_text(
            "<p>Hallo &amp; <b>Welt</b>,</p><p>Zeile 1<br>Zeile 2</p><ul><li>eins</li><li>zwei</li></ul>\
             <div class=\"quote-header\">Am X schrieb Y:</div><blockquote><div>Wer bringt<br>was mit?</div></blockquote><style>p{}</style>",
        );
        assert_eq!(
            t,
            "Hallo & Welt,\n\nZeile 1\nZeile 2\n- eins\n- zwei\nAm X schrieb Y:\n> Wer bringt\n> was mit?"
        );
    }

    #[test]
    fn reply_headers_and_message_id() {
        let h = ReplyHeaders::from_original(Some("<orig@x>"), Some("root@x"));
        assert_eq!(h.in_reply_to.as_deref(), Some("orig@x"));
        assert_eq!(h.references.as_deref(), Some("root@x orig@x"));
        let h = ReplyHeaders::from_original(None, None);
        assert_eq!(h, ReplyHeaders::default());

        let id = new_message_id("me@mail.de", "Test");
        assert!(id.starts_with('<') && id.ends_with("@mail.de>"));
        assert_ne!(id, new_message_id("me@mail.de", "Test2"));
    }

    #[test]
    fn builds_multipart_message_with_headers() {
        let reply = ReplyHeaders::from_original(Some("orig@x"), Some("root@x"));
        let to = vec![Recipient {
            name: Some("Tom \"T\" <x>".into()),
            address: "tom@example.org".into(),
        }];
        let (msg, id) = build(&Outgoing {
            identity: &identity(),
            to: &to,
            cc: &[],
            bcc: &[Recipient {
                name: None,
                address: "me@mail.de".into(),
            }],
            subject: "Re: Sommerfest",
            body_html: "<div>Ich bringe <b>Salat</b>.<script>x()</script></div>",
            reply: &reply,
            attachments: vec![OutgoingAttachment {
                name: "liste.txt".into(),
                mime: "text/plain".into(),
                bytes: b"Salat".to_vec(),
            }],
        })
        .unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(raw.contains("From: Adrian <me@mail.de>"));
        assert!(raw.contains("To: \"Tom T x\" <tom@example.org>"));
        assert!(!raw.contains("Bcc:"), "Bcc steht nicht im Kopf");
        assert!(raw.contains(&format!("Message-ID: {id}")));
        assert!(raw.contains("In-Reply-To: <orig@x>"));
        assert!(raw.contains("References: <root@x> <orig@x>"));
        assert!(raw.contains("Content-Type: multipart/mixed"));
        assert!(raw.contains("Content-Type: multipart/alternative"));
        assert!(raw.contains("Ich bringe Salat."));
        assert!(raw.contains("<b>Salat</b>"));
        assert!(!raw.contains("<script"));
        assert!(raw.contains("filename=\"liste.txt\""));
        assert!(
            build(&Outgoing {
                to: &[],
                bcc: &[],
                ..Outgoing {
                    identity: &identity(),
                    to: &to,
                    cc: &[],
                    bcc: &[],
                    subject: "",
                    body_html: "",
                    reply: &reply,
                    attachments: vec![],
                }
            })
            .is_err()
        );
    }
}
