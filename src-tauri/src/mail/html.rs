//! HTML-Mails für die Anzeige bereinigen (Regel 6).
//!
//! Drei Schichten schützen die Anzeige: diese Bereinigung, der `sandbox`-iframe
//! ohne `allow-scripts` im Frontend und eine eigene CSP im Dokument. Hier
//! werden Skripte, Formulare, eingebettete Rahmen und Ereignis-Attribute
//! entfernt, `javascript:`-Links neutralisiert, eingebettete Bilder (`cid:`)
//! als `data:`-URL eingesetzt und externe Bilder ohne Freigabe blockiert.

use std::cell::Cell;
use std::collections::HashMap;

use base64::Engine;
use lol_html::{RewriteStrSettings, element};

use crate::error::{Error, Result};

/// Elemente, die samt Inhalt verschwinden.
const DROP_WITH_CONTENT: &str = "script, style[media~='print'], iframe, frame, frameset, object, embed, applet, \
     form, input, button, select, textarea, noscript, template, svg script, meta[http-equiv], base, link";

/// Attribute, die auf jedem Element Netz- oder Codezugriff erlauben könnten.
const URL_ATTRIBUTES: &[&str] = &[
    "src",
    "href",
    "xlink:href",
    "poster",
    "background",
    "action",
    "formaction",
    "data",
    "srcset",
    "ping",
    "longdesc",
    "usemap",
];

/// Eingebettetes Bild: Content-ID → (MIME-Typ, Bytes).
pub type InlineImages<'a> = HashMap<String, (&'a str, &'a [u8])>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sanitized {
    /// vollständiges Dokument mit eigener CSP, für `srcdoc`
    pub html: String,
    /// externe Bilder, die entfernt wurden (0 bei Freigabe)
    pub blocked_images: usize,
}

/// CSP im Mail-Dokument: kein Skript, keine Verbindungen, Bilder nur
/// eingebettet, mit Freigabe zusätzlich per https.
fn csp(allow_remote: bool) -> String {
    let img = if allow_remote {
        "data: https:"
    } else {
        "data:"
    };
    format!(
        "default-src 'none'; img-src {img}; style-src 'unsafe-inline'; font-src data:; \
         form-action 'none'; base-uri 'none'; frame-ancestors 'none'"
    )
}

fn is_remote(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("http:") || u.starts_with("https:") || u.starts_with("//")
}

fn is_safe_link(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("http:")
        || u.starts_with("https:")
        || u.starts_with("mailto:")
        || u.starts_with('#')
}

/// Nur noch Bilder: data:, cid: (wird ersetzt) und mit Freigabe https.
fn image_allowed(url: &str, allow_remote: bool) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("data:image/") || (allow_remote && u.starts_with("https:"))
}

/// `url(...)`-Verweise in Inline-Styles: nur data: bleibt, sonst weg.
fn strip_style_urls(style: &str, allow_remote: bool) -> String {
    let lower = style.to_ascii_lowercase();
    let mut out = String::with_capacity(style.len());
    let mut rest = 0;
    while let Some(pos) = lower[rest..].find("url(") {
        let start = rest + pos;
        let Some(end_rel) = lower[start..].find(')') else {
            break;
        };
        let end = start + end_rel + 1;
        let inner = style[start + 4..end - 1]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        out.push_str(&style[rest..start]);
        if image_allowed(inner, allow_remote) {
            out.push_str(&style[start..end]);
        } else {
            out.push_str("none");
        }
        rest = end;
    }
    out.push_str(&style[rest..]);
    // @import, expression() und behavior: haben in Mails nichts zu suchen.
    if lower.contains("@import") || lower.contains("expression(") || lower.contains("behavior:") {
        return String::new();
    }
    out
}

pub fn sanitize(html: &str, inline: &InlineImages<'_>, allow_remote: bool) -> Result<Sanitized> {
    let blocked = Cell::new(0usize);
    let data_url = |cid: &str| -> Option<String> {
        let key = cid
            .trim()
            .trim_start_matches("cid:")
            .trim_matches(|c| c == '<' || c == '>');
        inline.get(key).map(|(mime, bytes)| {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        })
    };

    let handlers = vec![
        element!(DROP_WITH_CONTENT, |el| {
            el.remove();
            Ok(())
        }),
        // head-Reste wie <title> stören in srcdoc nicht; html/body
        // bleiben, damit Hintergrundfarben der Mail erhalten bleiben.
        element!("*", |el| {
            let names: Vec<String> = el.attributes().iter().map(|a| a.name()).collect();
            for name in names {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("on") || lower == "contenteditable" {
                    el.remove_attribute(&name);
                } else if lower == "style" {
                    let value = el.get_attribute(&name).unwrap_or_default();
                    let clean = strip_style_urls(&value, allow_remote);
                    if clean.len() != value.len() {
                        blocked.set(blocked.get() + 1);
                    }
                    el.set_attribute(&name, &clean).ok();
                } else if URL_ATTRIBUTES.contains(&lower.as_str())
                    && lower != "src"
                    && lower != "href"
                {
                    el.remove_attribute(&name);
                }
            }
            Ok(())
        }),
        element!("a[href]", |el| {
            let href = el.get_attribute("href").unwrap_or_default();
            if is_safe_link(&href) {
                el.set_attribute("rel", "noreferrer noopener").ok();
                el.set_attribute("target", "_blank").ok();
            } else {
                el.remove_attribute("href");
            }
            Ok(())
        }),
        element!("[href]:not(a)", |el| {
            el.remove_attribute("href");
            Ok(())
        }),
        element!(
            "img[src], video[src], audio[src], source[src], track[src]",
            |el| {
                let src = el.get_attribute("src").unwrap_or_default();
                let tag = el.tag_name();
                if tag != "img" {
                    el.remove();
                    return Ok(());
                }
                if let Some(data) = data_url(&src) {
                    el.set_attribute("src", &data).ok();
                } else if image_allowed(&src, allow_remote) {
                    // bleibt
                } else {
                    if is_remote(&src) || src.trim_start().starts_with("cid:") {
                        blocked.set(blocked.get() + 1);
                    }
                    el.remove_attribute("src");
                    el.set_attribute("data-blocked", "1").ok();
                    if el.get_attribute("alt").is_none() {
                        el.set_attribute("alt", "[Bild blockiert]").ok();
                    }
                }
                el.remove_attribute("srcset");
                el.remove_attribute("loading");
                Ok(())
            }
        ),
        element!("[src]:not(img)", |el| {
            el.remove_attribute("src");
            Ok(())
        }),
        element!("[background]", |el| {
            el.remove_attribute("background");
            Ok(())
        }),
        element!("style", |el| {
            // Style-Blöcke bleiben, ohne externe Verweise.
            el.set_attribute("data-mail", "1").ok();
            Ok(())
        }),
    ];
    let settings = handlers
        .into_iter()
        .fold(RewriteStrSettings::new(), |s, h| {
            s.append_element_content_handler(h)
        });
    let body = lol_html::rewrite_str(html, settings)
        .map_err(|e| Error::Parse(format!("HTML konnte nicht bereinigt werden: {e}")))?;

    let body = strip_style_block_urls(&body, allow_remote, &blocked);

    Ok(Sanitized {
        html: format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
             <meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
             <meta name=\"referrer\" content=\"no-referrer\">\
             <style>html,body{{margin:0;padding:12px;font:14px/1.5 -apple-system,BlinkMacSystemFont,\
             'Segoe UI',system-ui,sans-serif;color:#1f2328;background:#fff}}\
             img{{max-width:100%;height:auto}}img[data-blocked]{{display:inline-block;min-width:16px;\
             min-height:16px;border:1px dashed #bbb;background:#f3f3f3;color:#777;font-size:11px}}\
             pre{{white-space:pre-wrap}}</style></head><body>{body}</body></html>",
            csp(allow_remote)
        ),
        blocked_images: blocked.get(),
    })
}

/// `<style>`-Blöcke nachbearbeiten: lol_html liefert deren Text nicht als
/// Element-Attribut, daher ein zweiter Durchlauf über den fertigen String.
fn strip_style_block_urls(html: &str, allow_remote: bool, blocked: &Cell<usize>) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut rest = 0;
    while let Some(pos) = lower[rest..].find("<style") {
        let start = rest + pos;
        let Some(open_end_rel) = lower[start..].find('>') else {
            break;
        };
        let content_start = start + open_end_rel + 1;
        let Some(close_rel) = lower[content_start..].find("</style") else {
            break;
        };
        let content_end = content_start + close_rel;
        out.push_str(&html[rest..content_start]);
        let css = &html[content_start..content_end];
        let clean = strip_style_urls(css, allow_remote);
        if clean.len() != css.len() {
            blocked.set(blocked.get() + 1);
        }
        out.push_str(&clean);
        rest = content_end;
    }
    out.push_str(&html[rest..]);
    out
}

/// Klartext als HTML-Dokument (für Mails ohne HTML-Teil), Links klickbar.
pub fn text_as_html(text: &str) -> Sanitized {
    let mut body = String::with_capacity(text.len() + 64);
    body.push_str("<pre>");
    for line in text.lines() {
        linkify_into(&mut body, line);
        body.push('\n');
    }
    body.push_str("</pre>");
    Sanitized {
        html: format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
             <meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
             <meta name=\"referrer\" content=\"no-referrer\">\
             <style>html,body{{margin:0;padding:12px;background:#fff;color:#1f2328}}\
             pre{{margin:0;font:14px/1.55 -apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;\
             white-space:pre-wrap;overflow-wrap:anywhere}}</style></head><body>{body}</body></html>",
            csp(false)
        ),
        blocked_images: 0,
    }
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// http(s)-URLs im Text als Links; umgebende Klammern und Satzzeichen bleiben außen.
fn linkify_into(out: &mut String, line: &str) {
    let mut rest = line;
    while let Some(pos) = rest
        .find("http")
        .filter(|&p| rest[p..].starts_with("http://") || rest[p..].starts_with("https://"))
    {
        escape_into(out, &rest[..pos]);
        let candidate = &rest[pos..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"')
            .unwrap_or(candidate.len());
        let mut url = &candidate[..end];
        while let Some(stripped) = url.strip_suffix(|c: char| {
            matches!(c, '.' | ',' | ';' | ':' | ')' | ']' | '!' | '?' | '\'')
        }) {
            url = stripped;
        }
        out.push_str("<a href=\"");
        escape_into(out, url);
        out.push_str("\" rel=\"noreferrer noopener\" target=\"_blank\">");
        escape_into(out, url);
        out.push_str("</a>");
        rest = &rest[pos + url.len()..];
    }
    escape_into(out, rest);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> Sanitized {
        sanitize(html, &HashMap::new(), false).unwrap()
    }

    #[test]
    fn removes_scripts_handlers_and_javascript_links() {
        let s = clean(
            r#"<p onclick="x()" style="color:red">Hi<script>alert(1)</script></p>
               <a href="javascript:alert(1)">bad</a><a href="https://ok.example">ok</a>
               <iframe src="https://evil"></iframe><form action="https://x"><input></form>
               <meta http-equiv="refresh" content="0;url=https://x"><base href="https://x/">"#,
        );
        assert!(!s.html.contains("<script"));
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("javascript:"));
        assert!(!s.html.contains("<iframe"));
        assert!(!s.html.contains("<form"));
        assert!(!s.html.contains("<input"));
        assert!(!s.html.contains("http-equiv=\"refresh\""));
        assert!(!s.html.contains("<base"));
        assert!(s.html.contains("style=\"color:red\""));
        assert!(
            s.html
                .contains(r#"href="https://ok.example" rel="noreferrer noopener" target="_blank""#)
        );
        assert!(s.html.contains("Content-Security-Policy"));
        assert!(s.html.contains("default-src 'none'"));
        assert!(s.html.contains("img-src data:;"));
    }

    #[test]
    fn blocks_remote_images_until_allowed() {
        let html = r#"<img src="https://t.example/pixel.gif" width="1"><img src="data:image/png;base64,AAAA">
                      <div style="background:url('https://t.example/bg.png')">x</div>
                      <style>body{background:url(https://t.example/b.png)}</style>"#;
        let blocked = clean(html);
        assert_eq!(blocked.blocked_images, 3);
        assert!(!blocked.html.contains("t.example"));
        assert!(blocked.html.contains("data-blocked=\"1\""));
        assert!(blocked.html.contains("data:image/png"));

        let allowed = sanitize(html, &HashMap::new(), true).unwrap();
        assert_eq!(allowed.blocked_images, 0);
        assert!(allowed.html.contains("src=\"https://t.example/pixel.gif\""));
        assert!(allowed.html.contains("url('https://t.example/bg.png')"));
        assert!(allowed.html.contains("img-src data: https:"));
        // http (unverschlüsselt) bleibt auch mit Freigabe draußen
        assert!(
            sanitize(r#"<img src="http://x/a.png">"#, &HashMap::new(), true)
                .unwrap()
                .blocked_images
                == 1
        );
    }

    #[test]
    fn inlines_cid_images() {
        let mut inline: InlineImages<'_> = HashMap::new();
        inline.insert("logo@x".into(), ("image/png", b"\x89PNG".as_slice()));
        let s = sanitize(
            r#"<img src="cid:logo@x"><img src="cid:missing@x">"#,
            &inline,
            false,
        )
        .unwrap();
        assert!(s.html.contains("src=\"data:image/png;base64,iVBORw==\""));
        assert_eq!(s.blocked_images, 1, "fehlende cid zählt als blockiert");
    }

    #[test]
    fn strips_dangerous_css() {
        assert_eq!(
            strip_style_urls("color:red; background:url(https://x/a.png)", false),
            "color:red; background:none"
        );
        assert_eq!(
            strip_style_urls("background:url(\"data:image/gif;base64,R0lG\")", false),
            "background:url(\"data:image/gif;base64,R0lG\")"
        );
        assert_eq!(strip_style_urls("@import url(https://x/a.css)", false), "");
        assert_eq!(strip_style_urls("width:expression(alert(1))", false), "");
    }

    #[test]
    fn text_becomes_html_with_links() {
        let s =
            text_as_html("Hallo <b>,\nsiehe https://example.org/a?b=1&c=2. Danke (http://x.y/z)!");
        assert!(s.html.contains("Hallo &lt;b&gt;,"));
        assert!(s.html.contains(r#"<a href="https://example.org/a?b=1&amp;c=2" rel="noreferrer noopener" target="_blank">https://example.org/a?b=1&amp;c=2</a>."#));
        assert!(s.html.contains(r#"(<a href="http://x.y/z" rel="noreferrer noopener" target="_blank">http://x.y/z</a>)!"#));
        assert!(s.html.contains("default-src 'none'"));
    }
}
