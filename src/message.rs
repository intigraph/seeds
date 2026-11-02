//! Crew mail as its own entity, outside the board graph (aegis-snq; design in
//! shantytown `docs/message-graph.md`).
//!
//! A message is a `schema:Message` in a per-ISO-week window graph,
//! `<project graph>/messages/<YYYY>-W<ww>`, chosen by its send time and never
//! moved. The window prefix is not under `#seeds-…`, so no board read,
//! snapshot, pendant or export ever sees it. A message carries no work; it may
//! point at seeds with `schema:about`.

use crate::model::{Fact, Obj};
use crate::vocab;
use sha2::{Digest, Sha256};

/// `note` (ordinary mail) or `alert` (an alert escalation).
pub const KINDS: [&str; 2] = ["note", "alert"];

/// Longest label preview, in characters; the full body is `schema:text`.
pub const LABEL_CHARS: usize = 120;

/// One message as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Stable identifier, `msg-<hex>`.
    pub id: String,
    /// Recipient principal name.
    pub recipient: String,
    /// Sender principal name, when known.
    pub sender: Option<String>,
    /// Send time, `xsd:dateTime` (UTC, `…Z`).
    pub sent_at: String,
    /// The body.
    pub text: String,
    /// One of [`KINDS`].
    pub kind: String,
    /// Caller's idempotence key, when given.
    pub delivery_key: Option<String>,
    /// Seed ids the message refers to.
    pub about: Vec<String>,
}

fn schema(local: &str) -> String {
    format!("{}{local}", vocab::SCHEMA)
}

fn seeds(local: &str) -> String {
    format!("{}{local}", vocab::SEEDS)
}

/// The id of a message: derived from the delivery key when one is given, so a
/// retried send names the same message; else from recipient, time and body.
pub fn message_id(delivery_key: Option<&str>, recipient: &str, sent_at: &str, text: &str) -> String {
    let mut h = Sha256::new();
    match delivery_key {
        Some(k) => {
            h.update(b"key\0");
            h.update(k.as_bytes());
        }
        None => {
            for part in [recipient, sent_at, text] {
                h.update(part.as_bytes());
                h.update(b"\0");
            }
        }
    }
    let hex: String = h.finalize().iter().take(10).map(|b| format!("{b:02x}")).collect();
    format!("msg-{hex}")
}

/// ISO-8601 (year, week) of a `YYYY-MM-DD…` instant, or `None` if the date
/// part is not a valid calendar date.
pub fn iso_week(instant: &str) -> Option<(i64, u32)> {
    let date = instant.get(..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let ordinal = (1..m).map(|mm| days_in_month(y, mm)).sum::<u32>() + d;
    let weekday = weekday(y, m, d); // 1 = Monday .. 7 = Sunday
    let week = (i64::from(ordinal) - i64::from(weekday) + 10) / 7;
    if week < 1 {
        return Some((y - 1, weeks_in_year(y - 1)));
    }
    if week > i64::from(weeks_in_year(y)) {
        return Some((y + 1, 1));
    }
    Some((y, week as u32))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// ISO weekday, 1 = Monday .. 7 = Sunday (days-from-civil).
fn weekday(y: i64, m: u32, d: u32) -> u32 {
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (i64::from(m) - 3) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468; // 1970-01-01 = 0, a Thursday
    ((days + 3).rem_euclid(7) + 1) as u32
}

fn weeks_in_year(y: i64) -> u32 {
    // A year has 53 ISO weeks when it starts on Thursday, or on Wednesday in a leap year.
    let jan1 = weekday(y, 1, 1);
    if jan1 == 4 || (jan1 == 3 && is_leap(y)) {
        53
    } else {
        52
    }
}

/// The window graph a message sent at `sent_at` belongs to.
pub fn window_graph(project_graph: &str, sent_at: &str) -> Option<String> {
    let (y, w) = iso_week(sent_at)?;
    Some(format!("{}/messages/{y}-W{w:02}", project_graph.trim_end_matches('/')))
}

/// Whether `graph` is a message window of `project_graph`.
pub fn is_window_of(project_graph: &str, graph: &str) -> bool {
    let prefix = format!("{}/messages/", project_graph.trim_end_matches('/'));
    graph.strip_prefix(&prefix).is_some_and(|w| {
        let b = w.as_bytes();
        b.len() == 8
            && b[..4].iter().all(u8::is_ascii_digit)
            && &b[4..6] == b"-W"
            && b[6..].iter().all(u8::is_ascii_digit)
    })
}

/// The message's IRI.
pub fn message_iri(id: &str) -> String {
    format!("{}message/{id}", vocab::SEEDS_BASE)
}

/// The label floor: the first line of the body, at most [`LABEL_CHARS`].
pub fn label_preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(LABEL_CHARS).collect();
    if first.chars().count() > LABEL_CHARS {
        out.push('…');
    }
    if out.is_empty() {
        out.push_str("(empty message)");
    }
    out
}

impl Message {
    /// What is wrong with this message, if anything (empty = writable).
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.recipient.trim().is_empty() {
            p.push("recipient is empty".to_string());
        }
        if self.text.is_empty() {
            p.push("text is empty".to_string());
        }
        if !KINDS.contains(&self.kind.as_str()) {
            p.push(format!("kind {:?} is not one of note, alert", self.kind));
        }
        if iso_week(&self.sent_at).is_none() || !self.sent_at.ends_with('Z') {
            p.push(format!("sent_at {:?} is not a UTC xsd:dateTime", self.sent_at));
        }
        p
    }

    /// The facts written for this message (subject: [`message_iri`]).
    pub fn facts(&self) -> Vec<Fact> {
        let mut f: Vec<Fact> = vec![
            (vocab::RDF_TYPE.into(), Obj::Iri(schema("Message"))),
            (schema("identifier"), Obj::Str(self.id.clone())),
            (vocab::RDFS_LABEL.into(), Obj::Str(label_preview(&self.text))),
            (schema("recipient"), Obj::Iri(vocab::principal_iri(&self.recipient))),
            (
                schema("dateSent"),
                Obj::Typed {
                    lexical: self.sent_at.clone(),
                    datatype: vocab::XSD_DATE_TIME.into(),
                },
            ),
            (schema("text"), Obj::Str(self.text.clone())),
            (seeds("messageKind"), Obj::Str(self.kind.clone())),
        ];
        if let Some(s) = &self.sender {
            f.push((schema("sender"), Obj::Iri(vocab::principal_iri(s))));
        }
        if let Some(k) = &self.delivery_key {
            f.push((seeds("deliveryKey"), Obj::Str(k.clone())));
        }
        for id in &self.about {
            f.push((schema("about"), Obj::Iri(vocab::item_iri(id))));
        }
        f
    }

    /// Rebuild a message from its facts; `None` when it is not a well-formed message.
    pub fn from_facts(facts: &[Fact]) -> Option<Message> {
        let (mut typed, mut id, mut recipient, mut sender) = (false, None, None, None);
        let (mut sent_at, mut text, mut kind, mut key) = (None, None, None, None);
        let mut about = Vec::new();
        for (p, o) in facts {
            match (p.as_str(), o) {
                (vocab::RDF_TYPE, Obj::Iri(c)) if *c == schema("Message") => typed = true,
                (p, Obj::Str(s)) if p == schema("identifier") => id = Some(s.clone()),
                (p, Obj::Iri(i)) if p == schema("recipient") => recipient = vocab::principal_name(i),
                (p, Obj::Iri(i)) if p == schema("sender") => sender = vocab::principal_name(i),
                (p, Obj::Typed { lexical, .. } | Obj::Str(lexical)) if p == schema("dateSent") => {
                    sent_at = Some(lexical.clone())
                }
                (p, Obj::Str(s)) if p == schema("text") => text = Some(s.clone()),
                (p, Obj::Str(s)) if p == seeds("messageKind") => kind = Some(s.clone()),
                (p, Obj::Str(s)) if p == seeds("deliveryKey") => key = Some(s.clone()),
                (p, Obj::Iri(i)) if p == schema("about") => {
                    if let Some(id) = vocab::item_id(i) {
                        about.push(id)
                    }
                }
                _ => {}
            }
        }
        about.sort();
        if !typed {
            return None;
        }
        Some(Message {
            id: id?,
            recipient: recipient?,
            sender,
            sent_at: sent_at?,
            text: text?,
            kind: kind.unwrap_or_else(|| "note".into()),
            delivery_key: key,
            about,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn iso_weeks_match_the_calendar_including_year_edges() {
        for (day, want) in [
            ("2026-11-02T04:00:00Z", (2026, 45)),
            ("2026-01-01T00:00:00Z", (2026, 1)), // Thursday
            ("2027-01-01T00:00:00Z", (2026, 53)), // Friday of 2026-W53
            ("2024-12-30T00:00:00Z", (2025, 1)), // Monday of 2025-W01
            ("2021-01-03T00:00:00Z", (2020, 53)),
            ("2024-02-29T00:00:00Z", (2024, 9)),
        ] {
            assert_eq!(iso_week(day), Some(want), "{day}");
        }
        for bad in ["2026-02-30T00:00:00Z", "2026-13-01T00:00:00Z", "nope", "2026-1-1"] {
            assert_eq!(iso_week(bad), None, "{bad}");
        }
    }

    #[test]
    fn window_names_are_per_week_under_the_project_graph_and_never_under_seeds_fragments() {
        let g = "https://seeds.local/project/aegis";
        let w = window_graph(g, "2026-11-02T04:00:00Z").unwrap();
        assert_eq!(w, "https://seeds.local/project/aegis/messages/2026-W45");
        assert!(is_window_of(g, &w));
        assert!(!w.contains('#'));
        assert!(!is_window_of(g, g));
        assert!(!is_window_of(g, "https://seeds.local/project/aegis#seeds-ephemeral"));
        assert!(!is_window_of(g, "https://seeds.local/project/aegis/messages/2026-W4"));
    }

    #[test]
    fn a_keyed_message_keeps_its_id_and_facts_round_trip() {
        let a = message_id(Some("k1"), "wu", "2026-11-02T04:00:00Z", "x");
        let b = message_id(Some("k1"), "tim", "2026-11-03T04:00:00Z", "y");
        assert_eq!(a, b);
        assert_ne!(a, message_id(Some("k2"), "wu", "2026-11-02T04:00:00Z", "x"));
        assert_ne!(
            message_id(None, "wu", "2026-11-02T04:00:00Z", "x"),
            message_id(None, "wu", "2026-11-02T04:00:00Z", "y")
        );
        let m = Message {
            id: a,
            recipient: "wu".into(),
            sender: Some("grant".into()),
            sent_at: "2026-11-02T04:00:00Z".into(),
            text: "line one\nline two".into(),
            kind: "note".into(),
            delivery_key: Some("k1".into()),
            about: vec!["aegis-snq".into()],
        };
        assert!(m.problems().is_empty());
        assert_eq!(Message::from_facts(&m.facts()), Some(m.clone()));
        assert!(m
            .facts()
            .contains(&(vocab::RDFS_LABEL.into(), Obj::Str("line one".into()))));
    }

    #[test]
    fn bad_messages_are_named_not_written() {
        let m = Message {
            id: "msg-x".into(),
            recipient: " ".into(),
            sender: None,
            sent_at: "2026-11-02 04:00:00".into(),
            text: String::new(),
            kind: "task".into(),
            delivery_key: None,
            about: vec![],
        };
        assert_eq!(m.problems().len(), 4);
    }
}
