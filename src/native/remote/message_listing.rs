//! Page subjects first: a server's implicit fact-row cap must never split a
//! message and turn an ordinary mailbox into an apparently corrupt record.
use super::*;

type Rows = Vec<BTreeMap<String, Obj>>;
const SUBJECT_PAGE: usize = 128;
const FACT_LIMIT: usize = 1000;
const MAX_PAGES: usize = 1024;

fn facts(
    window: &str,
    subjects: &[String],
    select: &mut impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<Rows> {
    let values = subjects
        .iter()
        .map(|s| format!("<{s}>"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = format!("SELECT ?m ?p ?o WHERE {{ VALUES ?m {{ {values} }} GRAPH <{window}> {{ ?m ?p ?o }} }} LIMIT {FACT_LIMIT}");
    let (rows, truncated) = select(&query)?;
    if truncated || rows.len() >= FACT_LIMIT {
        if subjects.len() <= 1 {
            return Err(SdError::failed(format!(
                "message fact coverage exceeded for {} in {window}",
                subjects[0]
            )));
        }
        let mid = subjects.len() / 2;
        let mut left = facts(window, &subjects[..mid], select)?;
        left.extend(facts(window, &subjects[mid..], select)?);
        return Ok(left);
    }
    Ok(rows)
}

pub(super) fn collect(
    windows: &[String],
    recipient: Option<&str>,
    mut select: impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<crate::message::Listing> {
    let mut out = crate::message::Listing::default();
    for window in windows {
        let mut after = String::new();
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let recipient = recipient
                .map(|r| {
                    format!(
                        " ; <{}recipient> <{}>",
                        vocab::SCHEMA,
                        vocab::principal_iri(r)
                    )
                })
                .unwrap_or_default();
            let cursor = if after.is_empty() {
                String::new()
            } else {
                format!("FILTER(STR(?m) > \"{}\")", escape_literal(&after))
            };
            let query = format!("SELECT DISTINCT ?m WHERE {{ GRAPH <{window}> {{ ?m a <{}Message>{recipient} . {cursor} }} }} ORDER BY STR(?m) LIMIT {SUBJECT_PAGE}", vocab::SCHEMA);
            let (rows, truncated) = select(&query)?;
            if truncated || rows.len() > SUBJECT_PAGE {
                return Err(SdError::failed(format!(
                    "incomplete message subject page in {window}"
                )));
            }
            let mut subjects = Vec::new();
            for row in &rows {
                let Some(Obj::Iri(m)) = row.get("m") else {
                    return Err(SdError::failed(format!(
                        "invalid message subject in {window}"
                    )));
                };
                if !m.starts_with(&format!("{}message/", vocab::SEEDS_BASE)) || m <= &after {
                    return Err(SdError::failed(format!(
                        "non-progressing message subject page in {window}"
                    )));
                }
                after = m.clone();
                subjects.push(m.clone());
            }
            if !subjects.is_empty() {
                let mut by: BTreeMap<String, Vec<Fact>> =
                    subjects.iter().map(|s| (s.clone(), Vec::new())).collect();
                for row in facts(window, &subjects, &mut select)? {
                    let (Some(Obj::Iri(m)), Some(Obj::Iri(p)), Some(o)) =
                        (row.get("m"), row.get("p"), row.get("o"))
                    else {
                        return Err(SdError::failed(format!(
                            "invalid message fact row in {window}"
                        )));
                    };
                    let entry = by
                        .get_mut(m)
                        .ok_or_else(|| SdError::failed("message fact escaped selected subjects"))?;
                    entry.push((p.clone(), o.clone()));
                }
                for (subject, facts) in by {
                    match crate::message::Message::from_facts(&facts) {
                        Some(m) if crate::message::message_iri(&m.id) == subject => {
                            out.messages.push(m)
                        }
                        _ => out.warnings.push(format!(
                            "message listing incomplete: {subject} in {window} is malformed; omitted from this listing"
                        )),
                    }
                }
            }
            if rows.len() < SUBJECT_PAGE {
                complete = true;
                break;
            }
        }
        if !complete {
            return Err(SdError::failed(format!(
                "message subject page budget reached in {window}"
            )));
        }
    }
    out.messages
        .sort_by(|a, b| (&a.sent_at, &a.id).cmp(&(&b.sent_at, &b.id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(n: usize) -> crate::message::Message {
        crate::message::Message {
            id: format!("msg-{n:020}"),
            recipient: "reader".into(),
            sender: Some("sender".into()),
            sent_at: "2026-11-02T00:00:00Z".into(),
            text: "message".into(),
            kind: "note".into(),
            delivery_key: Some(format!("key-{n}")),
            about: Vec::new(),
            read_at: None,
        }
    }

    #[test]
    fn silent_fact_cap_never_splits_a_message_or_hides_later_mail() {
        let window = "https://example.org/project/mail/messages/2026-W45".to_string();
        let expected: Vec<_> = (0..1300).map(message).collect();
        let subjects: Vec<_> = expected
            .iter()
            .map(|m| crate::message::message_iri(&m.id))
            .collect();
        let mut page = 0;
        let mut fact_splits = 0;
        let listing = collect(&[window], None, |query| {
            if query.starts_with("SELECT DISTINCT") {
                if page > 0 {
                    assert!(query.contains(&subjects[page - 1]));
                }
                let end = (page + SUBJECT_PAGE).min(subjects.len());
                let rows = subjects[page..end]
                    .iter()
                    .map(|s| BTreeMap::from([("m".into(), Obj::Iri(s.clone()))]))
                    .collect();
                page = end;
                return Ok((rows, false));
            }
            let mut rows = Vec::new();
            for (m, subject) in expected.iter().zip(&subjects) {
                if query.contains(&format!("<{subject}>")) {
                    for (p, o) in m.facts() {
                        rows.push(BTreeMap::from([
                            ("m".into(), Obj::Iri(subject.clone())),
                            ("p".into(), Obj::Iri(p)),
                            ("o".into(), o),
                        ]));
                    }
                }
            }
            if rows.len() >= FACT_LIMIT {
                fact_splits += 1;
            }
            rows.truncate(FACT_LIMIT); // Same silent cap as the observed W3C response.
            Ok((rows, false))
        })
        .unwrap();
        assert!(fact_splits > 0);
        assert_eq!(listing.messages, expected);
        assert!(listing.warnings.is_empty());
    }

    #[test]
    fn a_real_malformed_record_is_named_while_good_messages_survive() {
        let good = message(1);
        let bad = message(2);
        let ids = [
            crate::message::message_iri(&good.id),
            crate::message::message_iri(&bad.id),
        ];
        let listing = collect(
            &["https://example.org/project/mail/messages/2026-W45".into()],
            None,
            |query| {
                if query.starts_with("SELECT DISTINCT") {
                    return Ok((
                        ids.iter()
                            .map(|s| BTreeMap::from([("m".into(), Obj::Iri(s.clone()))]))
                            .collect(),
                        false,
                    ));
                }
                let mut rows = Vec::new();
                for (n, m) in [&good, &bad].iter().enumerate() {
                    for (p, o) in m.facts() {
                        if n == 1 && p == format!("{}recipient", vocab::SCHEMA) {
                            continue;
                        }
                        rows.push(BTreeMap::from([
                            ("m".into(), Obj::Iri(ids[n].clone())),
                            ("p".into(), Obj::Iri(p)),
                            ("o".into(), o),
                        ]));
                    }
                }
                Ok((rows, false))
            },
        )
        .unwrap();
        assert_eq!(listing.messages, vec![good]);
        assert_eq!(listing.warnings.len(), 1);
        assert!(listing.warnings[0].contains(&bad.id));
        assert!(listing.warnings[0].starts_with("message listing incomplete:"));
    }

    #[test]
    fn truncated_subjects_and_nonprogressing_pages_are_errors() {
        let window = ["https://example.org/project/mail/messages/2026-W45".into()];
        assert!(collect(&window, None, |_| Ok((Vec::new(), true))).is_err());
        let row = BTreeMap::from([(
            "m".into(),
            Obj::Iri(crate::message::message_iri("msg-repeat")),
        )]);
        assert!(collect(&window, None, |_| Ok((
            vec![row.clone(), row.clone()],
            false
        )))
        .is_err());
    }
}
