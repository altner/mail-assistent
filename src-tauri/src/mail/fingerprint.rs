//! Identität einer Mail über Postfächer und Ordner hinweg.
//!
//! Hauptweg: SHA-256 aus Message-ID + Absender. Ohne Message-ID:
//! SHA-256 aus Absender, Datum, Betreff und Textanfang. Ein Präfix trennt
//! beide Varianten, damit sie nie kollidieren.

use sha2::{Digest, Sha256};

use super::parse::ParsedMail;

/// So viele Zeichen des (normalisierten) Textes fließen in den Fallback ein.
pub const BODY_PREFIX_CHARS: usize = 500;

pub fn fingerprint(mail: &ParsedMail) -> String {
    let from = mail.from_addr.as_deref().unwrap_or("").trim().to_lowercase();
    let input = match mail.message_id.as_deref() {
        Some(id) => format!("mid\n{id}\n{from}"),
        None => format!(
            "nomid\n{from}\n{}\n{}\n{}",
            mail.date.as_deref().unwrap_or(""),
            mail.subject.as_deref().unwrap_or("").trim(),
            body_prefix(mail.body_text.as_deref().unwrap_or("")),
        ),
    };
    hex(&Sha256::digest(input.as_bytes()))
}

/// Notlösung für Mails, die sich nicht parsen lassen: Hash über die Rohdaten.
pub fn fingerprint_raw(raw: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"raw\n");
    hasher.update(raw);
    hex(&hasher.finalize())
}

/// Leerraum zusammenfassen (CRLF vs. LF, Umbrüche) und auf eine feste
/// Zeichenzahl kürzen.
fn body_prefix(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(BODY_PREFIX_CHARS)
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: Option<&str>, from: &str) -> ParsedMail {
        ParsedMail {
            message_id: id.map(str::to_string),
            from_addr: Some(from.to_string()),
            subject: Some("Betreff".into()),
            date: Some("2024-01-01T10:00:00Z".into()),
            body_text: Some("Hallo\r\nWelt".into()),
            ..Default::default()
        }
    }

    #[test]
    fn is_sha256_hex() {
        let fp = fingerprint(&mail(Some("a@b"), "x@y"));
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn known_vector() {
        // sha256("mid\na@b\nx@y")
        assert_eq!(
            fingerprint(&mail(Some("a@b"), "x@y")),
            hex(&Sha256::digest(b"mid\na@b\nx@y"))
        );
    }

    #[test]
    fn message_id_path_ignores_other_fields() {
        let a = mail(Some("id@host"), "x@y");
        let mut b = a.clone();
        b.subject = Some("AW: anderer Betreff".into());
        b.body_text = Some("anderer Text".into());
        b.date = None;
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn sender_is_case_insensitive_and_part_of_identity() {
        assert_eq!(fingerprint(&mail(Some("id"), "X@Y")), fingerprint(&mail(Some("id"), "x@y")));
        assert_ne!(fingerprint(&mail(Some("id"), "x@y")), fingerprint(&mail(Some("id"), "z@y")));
    }

    #[test]
    fn fallback_uses_date_subject_and_body() {
        let a = mail(None, "x@y");
        let mut crlf = a.clone();
        crlf.body_text = Some("Hallo\nWelt  ".into());
        assert_eq!(fingerprint(&a), fingerprint(&crlf), "Leerraum darf nicht zählen");

        for change in [
            |m: &mut ParsedMail| m.subject = Some("Anders".into()),
            |m: &mut ParsedMail| m.date = Some("2024-01-02T10:00:00Z".into()),
            |m: &mut ParsedMail| m.body_text = Some("Anderer Text".into()),
        ] {
            let mut b = a.clone();
            change(&mut b);
            assert_ne!(fingerprint(&a), fingerprint(&b));
        }
    }

    #[test]
    fn fallback_only_looks_at_body_prefix() {
        let mut a = mail(None, "x@y");
        let long = "a".repeat(BODY_PREFIX_CHARS);
        a.body_text = Some(format!("{long} Ende 1"));
        let mut b = a.clone();
        b.body_text = Some(format!("{long} Ende 2"));
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn variants_do_not_collide() {
        let with_id = mail(Some("x"), "x@y");
        let without = mail(None, "x@y");
        assert_ne!(fingerprint(&with_id), fingerprint(&without));
    }
}
