//! Konversationen nach JWZ (<https://www.jwz.org/doc/threading.html>).
//!
//! Schritt 1 von JWZ verknüpft jede Mail mit allen Message-IDs aus
//! `References` und `In-Reply-To`; fehlende Mails werden zu Platzhaltern.
//! Weil die Ansicht chronologisch ist und keinen Antwortbaum braucht, genügen
//! hier die Zusammenhangskomponenten dieses Graphen (Union-Find). Zirkuläre
//! oder widersprüchliche Header können so keine Endlosschleife auslösen.
//!
//! Schritt 5 (Betreff) ist bewusst enger als bei JWZ: Eine Konversation wird
//! nur an eine frühere mit gleichem Betreff angehängt, wenn ihre erste Mail
//! ein Antwort-Präfix trägt und höchstens [`SUBJECT_WINDOW_DAYS`] Tage nach
//! deren letzter Mail liegt. Sonst würden z. B. alle „Ihre Rechnung“ verschmelzen.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

pub const SUBJECT_WINDOW_DAYS: i64 = 60;

/// Präfixe für Antworten und Weiterleitungen (kleingeschrieben, ohne Doppelpunkt).
const REPLY_PREFIXES: &[&str] = &[
    "re", "aw", "antw", "antwort", "fwd", "fw", "wg", "wtr", "tr", "sv", "vs", "rif", "ref", "enc",
];

#[derive(Debug, Clone, Default)]
pub struct ThreadInput {
    pub id: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// Message-IDs durch Leerzeichen getrennt, wie in `messages.refs`
    pub refs: Option<String>,
    pub subject: Option<String>,
    /// RFC 3339
    pub date: Option<String>,
}

/// Betreff ohne Antwort-Präfixe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSubject {
    /// für die Anzeige, Leerraum zusammengefasst
    pub display: String,
    /// zum Vergleichen, kleingeschrieben
    pub key: String,
    pub is_reply: bool,
}

pub fn normalize_subject(subject: &str) -> NormalizedSubject {
    let mut rest = subject;
    let mut is_reply = false;
    while let Some(after) = strip_reply_prefix(rest) {
        rest = after;
        is_reply = true;
    }
    let display = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    NormalizedSubject {
        key: display.to_lowercase(),
        display,
        is_reply,
    }
}

/// „Re:“, „AW:“, „Fwd:“, „RE :“, „Re[2]:“, „Re(3):“, „Re^2:“ …
fn strip_reply_prefix(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let word_len = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map_or(s.len(), |(i, _)| i);
    if word_len == 0 || !REPLY_PREFIXES.contains(&s[..word_len].to_ascii_lowercase().as_str()) {
        return None;
    }
    let mut rest = &s[word_len..];
    for (open, close) in [("[", "]"), ("(", ")")] {
        if let Some(inner) = rest.strip_prefix(open)
            && let Some(end) = inner.find(close)
            && inner[..end].chars().all(|c| c.is_ascii_digit())
        {
            rest = &inner[end + close.len()..];
        }
    }
    if let Some(inner) = rest.strip_prefix('^') {
        rest = inner.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let rest = rest.trim_start_matches(' ');
    rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))
}

fn parse_date(date: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(date?)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn add(&mut self) -> usize {
        self.0.push(self.0.len());
        self.0.len() - 1
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

/// Sortierschlüssel: mit Datum zuerst, dann nach Datum, dann nach ID.
fn chrono_key(m: &ThreadInput) -> (bool, Option<DateTime<Utc>>, i64) {
    let date = parse_date(m.date.as_deref());
    (date.is_none(), date, m.id)
}

/// Eigene Message-ID plus alle IDs aus `References` und `In-Reply-To`.
fn linked_ids(m: &ThreadInput) -> impl Iterator<Item = &str> {
    let own = m.message_id.as_deref().into_iter();
    let refs = m.refs.as_deref().unwrap_or("").split_whitespace();
    let reply = m.in_reply_to.as_deref().unwrap_or("").split_whitespace();
    own.chain(refs).chain(reply).filter(|id| !id.is_empty())
}

/// Teilt die Mails eines Kontos in Konversationen auf. Jede Gruppe enthält
/// die Mail-IDs chronologisch (ohne Datum zuletzt); die Gruppen sind nach
/// ihrer kleinsten ID sortiert.
pub fn group(messages: &[ThreadInput]) -> Vec<Vec<i64>> {
    // Knoten 0..n sind die Mails, weitere Knoten die Message-IDs.
    let mut uf = UnionFind((0..messages.len()).collect());
    let mut by_message_id: HashMap<&str, usize> = HashMap::new();
    for (index, m) in messages.iter().enumerate() {
        for id in linked_ids(m) {
            let node = *by_message_id.entry(id).or_insert_with(|| uf.add());
            uf.union(index, node);
        }
    }

    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..messages.len() {
        components.entry(uf.find(index)).or_default().push(index);
    }
    let mut components: Vec<Vec<usize>> = components.into_values().collect();
    for c in &mut components {
        c.sort_by_key(|&i| chrono_key(&messages[i]));
    }

    let components = merge_by_subject(messages, components);

    let mut groups: Vec<Vec<i64>> = components
        .into_iter()
        .map(|c| c.into_iter().map(|i| messages[i].id).collect())
        .collect();
    groups.sort_by_key(|g| g.iter().copied().min());
    groups
}

/// JWZ Schritt 5, eingeschränkt (siehe Moduldoku).
fn merge_by_subject(messages: &[ThreadInput], components: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
    struct Candidate {
        component: usize,
        key: String,
        is_reply: bool,
        first: DateTime<Utc>,
        last: DateTime<Utc>,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (component, members) in components.iter().enumerate() {
        let root = &messages[members[0]];
        let Some(subject) = root.subject.as_deref() else {
            continue;
        };
        let dates: Vec<_> = members
            .iter()
            .filter_map(|&i| parse_date(messages[i].date.as_deref()))
            .collect();
        let (Some(&first), Some(&last)) = (dates.first(), dates.iter().max()) else {
            continue;
        };
        let subject = normalize_subject(subject);
        if subject.key.is_empty() {
            continue;
        }
        candidates.push(Candidate {
            component,
            key: subject.key,
            is_reply: subject.is_reply,
            first,
            last,
        });
    }
    candidates.sort_by(|a, b| a.key.cmp(&b.key).then(a.first.cmp(&b.first)));

    // Ziel je Komponente: in welche Komponente sie aufgeht.
    let mut target: Vec<usize> = (0..components.len()).collect();
    let window = Duration::days(SUBJECT_WINDOW_DAYS);
    let mut cluster: Option<(usize, &str, DateTime<Utc>)> = None;
    for c in &candidates {
        match cluster {
            Some((owner, key, last)) if key == c.key && c.is_reply && c.first - last <= window => {
                target[c.component] = owner;
                cluster = Some((owner, key, last.max(c.last)));
            }
            _ => cluster = Some((c.component, &c.key, c.last)),
        }
    }

    let mut merged: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (component, members) in components.into_iter().enumerate() {
        merged[target[component]].extend(members);
    }
    merged
        .into_iter()
        .filter(|m| !m.is_empty())
        .map(|mut m| {
            m.sort_by_key(|&i| chrono_key(&messages[i]));
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: i64, message_id: &str, subject: &str, date: &str) -> ThreadInput {
        ThreadInput {
            id,
            message_id: Some(message_id.into()),
            subject: Some(subject.into()),
            date: Some(format!("{date}T10:00:00Z")),
            ..Default::default()
        }
    }

    fn reply(mut m: ThreadInput, in_reply_to: &str, refs: &str) -> ThreadInput {
        m.in_reply_to = Some(in_reply_to.into());
        m.refs = (!refs.is_empty()).then(|| refs.into());
        m
    }

    #[test]
    fn normalizes_subjects() {
        let n = normalize_subject("Re: AW:  Fwd: Angebot   Sommer");
        assert_eq!(n.display, "Angebot Sommer");
        assert_eq!(n.key, "angebot sommer");
        assert!(n.is_reply);

        for s in [
            "RE : Test",
            "Re[2]: Test",
            "Re(3): Test",
            "Re^2: Test",
            "WG: Test",
            "aw:Test",
        ] {
            let n = normalize_subject(s);
            assert_eq!((n.display.as_str(), n.is_reply), ("Test", true), "{s}");
        }

        let plain = normalize_subject("Rechnung: März");
        assert_eq!(
            (plain.display.as_str(), plain.is_reply),
            ("Rechnung: März", false)
        );
        assert!(!normalize_subject("Reise: Berlin").is_reply);
        assert!(!normalize_subject("Retoure").is_reply);
        assert_eq!(normalize_subject("Re:").key, "");
    }

    #[test]
    fn links_chain_via_references_and_in_reply_to() {
        let a = mail(1, "a@x", "Plan", "2026-01-01");
        let b = reply(mail(2, "b@x", "Re: Plan", "2026-01-02"), "a@x", "");
        let c = reply(mail(3, "c@x", "Re: Plan", "2026-01-03"), "b@x", "a@x b@x");
        let other = mail(4, "d@x", "Anderes", "2026-01-02");
        assert_eq!(group(&[c, other, b, a]), vec![vec![1, 2, 3], vec![4]]);
    }

    #[test]
    fn missing_parent_still_groups_siblings() {
        // Beide antworten auf eine Mail, die nicht im Archiv liegt.
        let a = reply(mail(1, "a@x", "Frage", "2026-02-01"), "root@x", "root@x");
        let b = reply(mail(2, "b@x", "Andere Zeile", "2026-02-02"), "root@x", "");
        assert_eq!(group(&[a, b]), vec![vec![1, 2]]);
    }

    #[test]
    fn survives_reference_loops() {
        let a = reply(mail(1, "a@x", "Loop", "2026-03-01"), "b@x", "b@x");
        let b = reply(mail(2, "b@x", "Loop", "2026-03-02"), "a@x", "a@x");
        let c = reply(mail(3, "c@x", "Loop", "2026-03-03"), "c@x", "c@x");
        assert_eq!(group(&[a, b, c]), vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn subject_fallback_needs_reply_prefix_and_window() {
        let original = mail(1, "a@x", "Angebot", "2026-01-01");
        let late_reply = mail(2, "b@x", "Re: Angebot", "2026-01-05");
        let new_original = mail(3, "c@x", "Angebot", "2026-01-10");
        let too_late = mail(4, "d@x", "AW: angebot", "2026-06-01");
        assert_eq!(
            group(&[original, late_reply, new_original, too_late]),
            vec![vec![1, 2], vec![3], vec![4]]
        );
    }

    #[test]
    fn same_subject_without_prefix_stays_apart() {
        let a = mail(1, "a@x", "Ihre Rechnung", "2026-01-01");
        let b = mail(2, "b@x", "Ihre Rechnung", "2026-01-02");
        assert_eq!(group(&[a, b]), vec![vec![1], vec![2]]);
    }

    #[test]
    fn mails_without_headers_or_date_are_single_threads() {
        let bare = ThreadInput {
            id: 1,
            ..Default::default()
        };
        let mut undated = mail(2, "b@x", "Re: Angebot", "2026-01-01");
        undated.date = None;
        let dated = mail(3, "c@x", "Angebot", "2026-01-01");
        assert_eq!(
            group(&[bare, undated, dated]),
            vec![vec![1], vec![2], vec![3]]
        );
    }
}
