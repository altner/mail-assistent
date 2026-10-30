use chrono::{DateTime, SecondsFormat, Utc};
use mail_parser::MessageParser;
use serde::Serialize;

use crate::error::{Error, Result};

/// Die Felder einer Mail, die ins Archiv übernommen werden.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedMail {
    /// ohne spitze Klammern
    pub message_id: Option<String>,
    /// Message-IDs ohne Klammern, durch Leerzeichen getrennt
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    /// kleingeschrieben
    pub from_addr: Option<String>,
    pub from_name: Option<String>,
    pub to: Vec<Address>,
    pub subject: Option<String>,
    /// `Date`-Header als RFC 3339 in UTC
    pub date: Option<String>,
    /// Klartext; reine HTML-Mails werden vom Parser in Text umgewandelt
    pub body_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Address {
    pub name: Option<String>,
    pub address: Option<String>,
}

pub fn parse(raw: &[u8]) -> Result<ParsedMail> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Parse("kein gültiger Mail-Header".into()))?;

    let from = msg.from().and_then(|list| list.first());
    let to = msg
        .to()
        .map(|list| {
            list.mailboxes()
                .map(|m| Address {
                    name: non_empty(m.name()),
                    address: non_empty(m.address()).map(|a| a.to_lowercase()),
                })
                .collect()
        })
        .unwrap_or_default();

    let body: Vec<String> = msg.text_bodies().map(|t| t.into_owned()).collect();
    let body_text = non_empty(Some(body.join("\n\n").trim()));

    Ok(ParsedMail {
        message_id: msg.message_id().and_then(|id| normalize_message_id(id)),
        in_reply_to: msg.in_reply_to().and_then(|l| join_ids(l.iter())),
        references: msg.references().and_then(|l| join_ids(l.iter())),
        from_addr: from.and_then(|m| non_empty(m.address())).map(|a| a.to_lowercase()),
        from_name: from.and_then(|m| non_empty(m.name())),
        to,
        subject: non_empty(msg.subject()),
        date: msg.date().and_then(|d| timestamp_to_rfc3339(d.to_timestamp())),
        body_text,
    })
}

/// Entfernt Leerraum und spitze Klammern; leere IDs zählen als fehlend.
pub fn normalize_message_id(id: &str) -> Option<String> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!id.is_empty()).then(|| id.to_string())
}

pub fn timestamp_to_rfc3339(ts: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(ts, 0).map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn join_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Option<String> {
    let ids: Vec<String> = ids.filter_map(normalize_message_id).collect();
    (!ids.is_empty()).then(|| ids.join(" "))
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &[u8] = b"From: =?UTF-8?Q?J=C3=BCrgen_M=C3=BCller?= <Juergen@Example.ORG>\r\n\
To: Adrian <adrian@example.com>, info@example.net\r\n\
Subject: =?UTF-8?Q?Rechnung_f=C3=BCr_Oktober?=\r\n\
Date: Sat, 20 Nov 2021 14:22:01 -0800\r\n\
Message-ID: <abc.123@example.org>\r\n\
In-Reply-To: <parent@example.org>\r\n\
References: <root@example.org> <parent@example.org>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hallo Adrian,\r\nanbei die Rechnung.\r\n";

    #[test]
    fn parses_headers_and_body() {
        let m = parse(PLAIN).unwrap();
        assert_eq!(m.message_id.as_deref(), Some("abc.123@example.org"));
        assert_eq!(m.in_reply_to.as_deref(), Some("parent@example.org"));
        assert_eq!(m.references.as_deref(), Some("root@example.org parent@example.org"));
        assert_eq!(m.from_addr.as_deref(), Some("juergen@example.org"));
        assert_eq!(m.from_name.as_deref(), Some("Jürgen Müller"));
        assert_eq!(m.subject.as_deref(), Some("Rechnung für Oktober"));
        assert_eq!(m.date.as_deref(), Some("2021-11-20T22:22:01Z"));
        assert_eq!(m.to.len(), 2);
        assert_eq!(m.to[1].address.as_deref(), Some("info@example.net"));
        assert!(m.body_text.unwrap().starts_with("Hallo Adrian,"));
    }

    #[test]
    fn html_only_mail_gets_text_body() {
        let raw = b"From: a@example.org\r\nSubject: HTML\r\n\
Content-Type: text/html; charset=utf-8\r\n\r\n\
<html><body><p>Guten <b>Tag</b></p><script>alert(1)</script></body></html>\r\n";
        let m = parse(raw).unwrap();
        let body = m.body_text.unwrap();
        assert!(body.contains("Guten"), "body: {body}");
        assert!(!body.contains("<b>"), "body: {body}");
        assert!(m.message_id.is_none());
        assert!(m.date.is_none());
    }

    #[test]
    fn multipart_alternative_prefers_text_part() {
        let raw = b"From: a@example.org\r\nSubject: Multi\r\nMessage-ID: <m@x>\r\n\
Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nNur Text\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<p>HTML-Variante</p>\r\n--b--\r\n";
        let m = parse(raw).unwrap();
        assert_eq!(m.body_text.as_deref(), Some("Nur Text"));
    }

    #[test]
    fn empty_message_id_counts_as_missing() {
        assert_eq!(normalize_message_id("  <> "), None);
        assert_eq!(normalize_message_id(" <a@b> "), Some("a@b".into()));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"").is_err());
    }
}
