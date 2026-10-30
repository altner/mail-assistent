//! Anzeige-Inhalt einer archivierten Mail aus der `.eml`-Datei: bereinigtes
//! HTML (oder Klartext als HTML) und die Liste der Anhänge.

use std::collections::HashMap;

use mail_parser::{MessageParser, PartKind};
use serde::Serialize;

use super::html::{self, InlineImages, Sanitized};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    /// Index in der Teileliste der Mail; stabil, solange die Datei gleich ist
    pub part: usize,
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    /// vollständiges Dokument für `srcdoc`
    pub html: String,
    /// ob das Dokument aus dem HTML-Teil stammt (sonst Klartext)
    pub is_html: bool,
    pub blocked_images: usize,
    pub attachments: Vec<AttachmentInfo>,
}

fn parse(raw: &[u8]) -> Result<mail_parser::Message<'_>> {
    MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Parse("Mail-Datei nicht lesbar".into()))
}

/// Anhänge: alle nicht angezeigten Teile. Eingebettete Bilder (`cid:` mit
/// `inline`), die im HTML erscheinen, gehören nicht dazu.
pub fn attachments(raw: &[u8]) -> Result<Vec<AttachmentInfo>> {
    let msg = parse(raw)?;
    Ok(msg
        .attachments()
        .filter(|p| !matches!(p.kind(), PartKind::Multipart))
        .filter(|p| {
            let inline_image = p.content_id().is_some()
                && p.content_disposition().is_some_and(|d| d.is_inline())
                && p.content_type()
                    .is_some_and(|c| c.ctype().eq_ignore_ascii_case("image"));
            !inline_image
        })
        .filter_map(|p| {
            let (ctype, sub) = p
                .content_type()
                .map(|c| {
                    (
                        c.ctype().to_string(),
                        c.subtype().unwrap_or("octet-stream").to_string(),
                    )
                })
                .unwrap_or_else(|| ("application".into(), "octet-stream".into()));
            let mime = format!("{}/{}", ctype.to_lowercase(), sub.to_lowercase());
            let filename = p
                .attachment_name()
                .map(str::to_string)
                .unwrap_or_else(|| fallback_name(u32::from(p.id()) as usize, &mime));
            let size = p.decoded().len();
            Some(AttachmentInfo {
                part: u32::from(p.id()) as usize,
                filename,
                mime_type: mime,
                size,
            })
        })
        .collect())
}

fn fallback_name(part: usize, mime: &str) -> String {
    let ext = match mime {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "text/calendar" => "ics",
        "message/rfc822" => "eml",
        _ => "bin",
    };
    format!("anhang-{part}.{ext}")
}

/// Liefert den Inhalt eines Anhangs (Name, MIME-Typ, Bytes).
pub fn attachment_bytes(raw: &[u8], part: usize) -> Result<(String, String, Vec<u8>)> {
    let list = attachments(raw)?;
    let info = list
        .into_iter()
        .find(|a| a.part == part)
        .ok_or_else(|| Error::NotFound(format!("Anhang {part}")))?;
    let msg = parse(raw)?;
    let p = msg
        .parts()
        .nth(part)
        .ok_or_else(|| Error::NotFound(format!("Anhang {part}")))?;
    Ok((info.filename, info.mime_type, p.decoded().into_owned()))
}

/// Dateiname ohne Pfadanteile und Steuerzeichen, für den Speichern-Dialog.
pub fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "anhang".into()
    } else {
        trimmed.chars().take(200).collect()
    }
}

pub fn body(raw: &[u8], allow_remote: bool) -> Result<Body> {
    let msg = parse(raw)?;
    let attachments = attachments(raw)?;

    // Eingebettete Bilder nach Content-ID.
    let mut inline: InlineImages<'_> = HashMap::new();
    let mut inline_data: Vec<(String, String, Vec<u8>)> = Vec::new();
    for p in msg.parts() {
        if matches!(
            p.kind(),
            PartKind::Multipart | PartKind::Text | PartKind::Html
        ) {
            continue;
        }
        let Some(cid) = p.content_id() else { continue };
        let mime = p
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        if !mime.to_lowercase().starts_with("image/") {
            continue;
        }
        inline_data.push((
            cid.to_string(),
            mime.to_lowercase(),
            p.decoded().into_owned(),
        ));
    }
    for (cid, mime, bytes) in &inline_data {
        inline.insert(cid.clone(), (mime.as_str(), bytes.as_slice()));
    }

    // html_bodies() liefert auch Textteile als Ersatz; echtes HTML nur, wenn
    // mindestens ein Teil wirklich text/html ist.
    let has_html = msg.html_body().any(|p| matches!(p.kind(), PartKind::Html));
    let html_parts: Vec<String> = msg.html_bodies().map(|c| c.into_owned()).collect();
    let (sanitized, is_html): (Sanitized, bool) = if has_html && !html_parts.is_empty() {
        (
            html::sanitize(&html_parts.join("<hr>"), &inline, allow_remote)?,
            true,
        )
    } else {
        let text: Vec<String> = msg.text_bodies().map(|c| c.into_owned()).collect();
        (html::text_as_html(&text.join("\n\n")), false)
    };

    Ok(Body {
        html: sanitized.html,
        is_html,
        blocked_images: sanitized.blocked_images,
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTIPART: &str = "From: a@x\r\nTo: b@x\r\nSubject: Test\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
--outer\r\nContent-Type: multipart/related; boundary=\"rel\"\r\n\r\n\
--rel\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n\
--alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHallo Text\r\n\
--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hallo <img src=\"cid:logo@x\"><img src=\"https://t.example/p.gif\"><script>x</script></p>\r\n\
--alt--\r\n\
--rel\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw==\r\n\
--rel--\r\n\
--outer\r\nContent-Type: application/pdf; name=\"Rechnung.pdf\"\r\nContent-Disposition: attachment; filename=\"Rechnung.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n\
--outer--\r\n";

    const PLAIN: &str = "From: a@x\r\nSubject: Nur Text\r\n\r\nSiehe https://example.org/x.\r\n";

    #[test]
    fn html_body_with_inline_image_and_blocked_tracker() {
        let b = body(MULTIPART.as_bytes(), false).unwrap();
        assert!(b.is_html);
        assert!(b.html.contains("src=\"data:image/png;base64,iVBORw==\""));
        assert!(!b.html.contains("t.example"));
        assert!(!b.html.contains("<script"));
        assert_eq!(b.blocked_images, 1);
        assert_eq!(
            b.attachments,
            vec![AttachmentInfo {
                part: 6,
                filename: "Rechnung.pdf".into(),
                mime_type: "application/pdf".into(),
                size: 5,
            }]
        );
    }

    #[test]
    fn attachment_bytes_by_part() {
        let (name, mime, bytes) = attachment_bytes(MULTIPART.as_bytes(), 6).unwrap();
        assert_eq!(
            (name.as_str(), mime.as_str()),
            ("Rechnung.pdf", "application/pdf")
        );
        assert_eq!(bytes, b"%PDF-");
        assert!(
            attachment_bytes(MULTIPART.as_bytes(), 2).is_err(),
            "Textteil ist kein Anhang"
        );
    }

    #[test]
    fn plain_text_mail_gets_linkified_document() {
        let b = body(PLAIN.as_bytes(), false).unwrap();
        assert!(!b.is_html);
        assert!(b.html.contains("<a href=\"https://example.org/x\""));
        assert!(b.attachments.is_empty());
    }

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(safe_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_filename("  .hidden "), "hidden");
        assert_eq!(safe_filename(""), "anhang");
        assert_eq!(safe_filename("Rechnung 2026.pdf"), "Rechnung 2026.pdf");
    }
}
