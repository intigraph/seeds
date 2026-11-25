//! Page subjects first: a server's implicit fact-row cap must never split a
//! message and turn an ordinary mailbox into an apparently corrupt record.
use super::*;

type Rows = Vec<BTreeMap<String, Obj>>;
const SUBJECT_PAGE: usize = 128;
const FACT_LIMIT: usize = 1000;
const MAX_PAGES: usize = 1024;

// Historical windows usually contain no live mail for this recipient. Prove
// that in bounded groups instead of spending one empty subject query per week.
// This is a fresh scoped read, never a recent-window shortcut or receipt cache.
fn populated_windows(
    windows: &[String],
    recipient: &str,
    sent_filter: &str,
    select: &mut impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<Vec<String>> {
    if windows.len() <= 1 {
        return Ok(windows.to_vec());
    }
    let mut populated = BTreeSet::new();
    for chunk in windows.chunks(64) {
        let values = chunk
            .iter()
            .map(|w| format!("<{w}>"))
            .collect::<Vec<_>>()
            .join(" ");
        let query = format!("SELECT DISTINCT ?window WHERE {{ VALUES ?window {{ {values} }} GRAPH ?window {{ ?m a <{}Message>{recipient}{sent_filter} . }} }} LIMIT {}", vocab::SCHEMA, chunk.len() + 1);
        let (rows, truncated) = select(&query)?;
        if truncated || rows.len() > chunk.len() {
            return Err(SdError::failed(
                "incomplete message window presence coverage",
            ));
        }
        for row in rows {
            let Some(Obj::Iri(window)) = row.get("window") else {
                return Err(SdError::failed("invalid message window presence row"));
            };
            if !chunk.contains(window) || !populated.insert(window.clone()) {
                return Err(SdError::failed(
                    "message window presence escaped scope or repeated",
                ));
            }
        }
    }
    Ok(windows
        .iter()
        .filter(|w| populated.contains(*w))
        .cloned()
        .collect())
}

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
    since: Option<&str>,
    select: impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<crate::message::Listing> {
    collect_selected(windows, recipient, since, false, select)
}

/// Only messages without a literal `schema:dateRead` in their window, which is
/// exactly what `Message::from_facts` leaves with `read_at: None` (aegis-jve).
/// The read predicate runs in the presence and subject queries, so marked mail
/// is never paged or hydrated just to be discarded by the caller. Every selected
/// subject is still hydrated and validated; a malformed unread record still
/// surfaces as a warning.
pub(super) fn collect_unread(
    windows: &[String],
    recipient: Option<&str>,
    since: Option<&str>,
    select: impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<crate::message::Listing> {
    collect_selected(windows, recipient, since, true, select)
}

fn collect_selected(
    windows: &[String],
    recipient: Option<&str>,
    since: Option<&str>,
    unread: bool,
    mut select: impl FnMut(&str) -> Result<(Rows, bool)>,
) -> Result<crate::message::Listing> {
    let mut out = crate::message::Listing::default();
    // aegis-9qx: the TTL cutoff runs in the subject query, so an expired
    // message is never paged out of the server. Only the cutoff instant (an
    // xsd:dateTime the client computed) enters the query.
    let sent_filter = since
        .map(|cutoff| {
            format!(
                " ; <{}dateSent> ?sent . FILTER(?sent >= \"{}\"^^<{}>)",
                vocab::SCHEMA,
                escape_literal(cutoff),
                vocab::XSD_DATE_TIME
            )
        })
        .unwrap_or_default();
    let recipient = recipient
        .map(|r| {
            format!(
                " ; <{}recipient> <{}>",
                vocab::SCHEMA,
                vocab::principal_iri(r)
            )
        })
        .unwrap_or_default();
    let sent_filter = if unread {
        format!(
            "{sent_filter} . FILTER NOT EXISTS {{ ?m <{}dateRead> ?read . FILTER(isLiteral(?read)) }}",
            vocab::SCHEMA
        )
    } else {
        sent_filter
    };
    let windows = populated_windows(windows, &recipient, &sent_filter, &mut select)?;
    for window in &windows {
        let mut after = String::new();
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let cursor = if after.is_empty() {
                String::new()
            } else {
                format!("FILTER(STR(?m) > \"{}\")", escape_literal(&after))
            };
            let query = format!("SELECT DISTINCT ?m WHERE {{ GRAPH <{window}> {{ ?m a <{}Message>{recipient}{sent_filter} . {cursor} }} }} ORDER BY STR(?m) LIMIT {SUBJECT_PAGE}", vocab::SCHEMA);
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
                            if !since.is_some_and(|c| crate::message::is_expired(&m, c))
                                && !(unread && m.read_at.is_some())
                            {
                                out.messages.push(m)
                            }
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

    #[test]
    fn empty_history_is_one_scoped_probe_not_one_query_per_window() {
        let windows: Vec<_> = (1..=20)
            .map(|n| format!("https://example.org/mail/{n}"))
            .collect();
        let mut calls = 0;
        let listing = collect(
            &windows,
            Some("reader"),
            Some("2026-11-13T06:00:00Z"),
            |q| {
                calls += 1;
                assert!(q.starts_with("SELECT DISTINCT ?window"), "{q}");
                for w in &windows {
                    assert!(q.contains(&format!("<{w}>")));
                }
                assert!(q.contains(&vocab::principal_iri("reader")));
                assert!(q.contains("FILTER(?sent >="));
                Ok((vec![], false))
            },
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert!(listing.messages.is_empty() && listing.warnings.is_empty());
    }

    #[test]
    fn unread_pushes_the_read_predicate_into_presence_and_subject_queries() {
        let windows: Vec<String> = vec![
            "https://example.org/mail/a".into(),
            "https://example.org/mail/b".into(),
        ];
        let not_read = format!(
            "FILTER NOT EXISTS {{ ?m <{}dateRead> ?read . FILTER(isLiteral(?read)) }}",
            vocab::SCHEMA
        );
        let mut seen = Vec::new();
        let listing = collect_unread(
            &windows,
            Some("reader"),
            Some("2026-11-13T06:00:00Z"),
            |q| {
                seen.push(q.to_string());
                assert!(q.contains(&not_read), "{q}");
                assert!(q.contains("FILTER(?sent >="), "{q}");
                if q.starts_with("SELECT DISTINCT ?window") {
                    return Ok((
                        vec![BTreeMap::from([(
                            "window".into(),
                            Obj::Iri(windows[1].clone()),
                        )])],
                        false,
                    ));
                }
                assert!(q.starts_with("SELECT DISTINCT ?m"), "{q}");
                assert!(q.contains(&format!("GRAPH <{}>", windows[1])), "{q}");
                Ok((vec![], false))
            },
        )
        .unwrap();
        assert_eq!(seen.len(), 2);
        assert!(listing.messages.is_empty() && listing.warnings.is_empty());
        // The ordinary listing keeps read mail: no read predicate.
        collect(&windows, Some("reader"), None, |q| {
            assert!(!q.contains("dateRead"), "{q}");
            Ok((vec![], false))
        })
        .unwrap();
    }

    #[test]
    fn presence_refuses_unknown_duplicate_truncated_and_malformed_rows() {
        let windows: Vec<String> = vec![
            "https://example.org/a".into(),
            "https://example.org/b".into(),
        ];
        let row = |w: &str| BTreeMap::from([("window".into(), Obj::Iri(w.into()))]);
        for (rows, truncated) in [
            (vec![], true),
            (vec![BTreeMap::new()], false),
            (vec![row("https://example.org/outside")], false),
            (vec![row(&windows[0]), row(&windows[0])], false),
        ] {
            assert!(collect(&windows, None, None, |_| Ok((rows.clone(), truncated))).is_err());
        }
    }

    #[test]
    fn presence_is_bounded_and_keeps_nonempty_windows_in_caller_order() {
        let windows: Vec<_> = (0..130)
            .map(|n| format!("https://example.org/mail/{n}"))
            .collect();
        let mut probes = 0;
        let got = populated_windows(&windows, "", "", &mut |q| {
            let chunk = &windows[probes * 64..((probes + 1) * 64).min(windows.len())];
            probes += 1;
            assert!(q.contains(&format!("LIMIT {}", chunk.len() + 1)));
            Ok((
                chunk
                    .iter()
                    .rev()
                    .map(|w| BTreeMap::from([("window".into(), Obj::Iri(w.clone()))]))
                    .collect(),
                false,
            ))
        })
        .unwrap();
        assert_eq!(probes, 3);
        assert_eq!(got, windows);
    }

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
        let listing = collect(&[window], None, None, |query| {
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
    fn a_ttl_cutoff_filters_in_the_subject_query_and_again_on_the_client() {
        // aegis-9qx: the server is asked only for messages sent at or after the
        // cutoff. A server that ignored the filter still cannot leak an expired
        // message into the listing.
        let mut fresh = message(1);
        fresh.sent_at = "2026-11-19T00:00:00Z".into();
        let mut old = message(2);
        old.sent_at = "2026-11-12T00:00:00Z".into();
        let ids = [
            crate::message::message_iri(&fresh.id),
            crate::message::message_iri(&old.id),
        ];
        let mut subject_queries = Vec::new();
        let listing = collect(
            &["https://example.org/project/mail/messages/2026-W47".into()],
            Some("reader"),
            Some("2026-11-13T06:00:00Z"),
            |query| {
                if query.starts_with("SELECT DISTINCT") {
                    subject_queries.push(query.to_string());
                    return Ok((
                        ids.iter()
                            .map(|s| BTreeMap::from([("m".into(), Obj::Iri(s.clone()))]))
                            .collect(),
                        false,
                    ));
                }
                let mut rows = Vec::new();
                for (n, m) in [&fresh, &old].iter().enumerate() {
                    for (p, o) in m.facts() {
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
        assert_eq!(listing.messages, vec![fresh]);
        assert!(listing.warnings.is_empty());
        let q = &subject_queries[0];
        assert!(
            q.contains(&format!("<{}dateSent> ?sent", vocab::SCHEMA)),
            "{q}"
        );
        assert!(
            q.contains(&format!(
                "FILTER(?sent >= \"2026-11-13T06:00:00Z\"^^<{}>)",
                vocab::XSD_DATE_TIME
            )),
            "{q}"
        );
        // Without a cutoff the subject query is unchanged.
        let mut plain = Vec::new();
        collect(
            &["https://example.org/project/mail/messages/2026-W47".into()],
            None,
            None,
            |query| {
                plain.push(query.to_string());
                Ok((Vec::new(), false))
            },
        )
        .unwrap();
        assert!(!plain[0].contains("dateSent"), "{}", plain[0]);
    }

    #[test]
    fn truncated_subjects_and_nonprogressing_pages_are_errors() {
        let window = ["https://example.org/project/mail/messages/2026-W45".into()];
        assert!(collect(&window, None, None, |_| Ok((Vec::new(), true))).is_err());
        let row = BTreeMap::from([(
            "m".into(),
            Obj::Iri(crate::message::message_iri("msg-repeat")),
        )]);
        assert!(collect(&window, None, None, |_| Ok((
            vec![row.clone(), row.clone()],
            false
        )))
        .is_err());
    }
}
