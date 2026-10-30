//! Dekodierung von IMAP-Ordnernamen (modifiziertes UTF-7, RFC 3501 §5.1.3),
//! z. B. `Entw&APw-rfe` → `Entwürfe`. Nur für Anzeige und Namensvergleich;
//! gegenüber dem Server wird immer der Originalname verwendet.

pub fn decode(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some(len) = bytes[i + 1..].iter().position(|&c| c == b'-')
        {
            let chunk = &name[i + 1..i + 1 + len];
            match decode_chunk(chunk) {
                Some(text) => out.push_str(&text),
                None => out.push_str(&name[i..i + len + 2]),
            }
            i += len + 2;
            continue;
        }
        let ch = name[i..].chars().next().expect("gültiger UTF-8-Index");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_chunk(chunk: &str) -> Option<String> {
    if chunk.is_empty() {
        return Some("&".into());
    }
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::new();
    for c in chunk.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b',' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_umlauts_and_ampersand() {
        assert_eq!(decode("INBOX"), "INBOX");
        assert_eq!(decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode("Gel&APY-schte Elemente"), "Gelöschte Elemente");
        assert_eq!(decode("A &- B"), "A & B");
        assert_eq!(decode("&ZeVnLIqe-"), "日本語");
    }

    #[test]
    fn keeps_invalid_sequences() {
        assert_eq!(decode("kaputt&!!-"), "kaputt&!!-");
        assert_eq!(decode("offen&AB"), "offen&AB");
    }
}
