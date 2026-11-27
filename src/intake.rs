//! Duplicate intake reuses Quipu's episode resolver, without writing the board.
use crate::error::{Result, SdError};
use crate::model::Seed;
use quipu::resolution::EntityCandidate;
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};

/// Embedding similarity; a score is not a probability.
pub const THRESHOLD: f64 = 0.92;
/// Long tracker templates can have high Jaro scores despite different targets.
/// The retained replay flagged 119/533 at .92, including different episode IDs;
/// .99 retained 88, with those inspected template-only matches removed.
pub const NAME_THRESHOLD: f64 = 0.99;
/// A full result at this bound is incomplete and must not authorize creation.
pub const CANDIDATE_CAP: usize = 1000;

/// Local/offline name matching over exactly the supplied active corpus. The
/// scratch store has no embedding provider; remote intake adds the configured
/// server's embedding half. Never represent this as embedding coverage.
pub fn name_candidates<'a>(
    seeds: impl IntoIterator<Item = &'a Seed>,
    title: &str,
    description: Option<&str>,
) -> Result<Vec<EntityCandidate>> {
    let mut store = Store::open_in_memory().map_err(graph_error)?;
    let label = store
        .intern(crate::vocab::RDFS_LABEL)
        .map_err(graph_error)?;
    let mut facts = Vec::new();
    for seed in seeds {
        if matches!(seed.status.as_str(), "closed" | "tombstone") {
            continue;
        }
        facts.push(Datum {
            entity: store
                .intern(&crate::vocab::item_iri(&seed.id))
                .map_err(graph_error)?,
            attribute: label,
            value: Value::Str(seed.title.clone()),
            valid_from: "1970-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    if !facts.is_empty() {
        store
            .transact(
                &facts,
                "1970-01-01T00:00:00Z",
                None,
                Some("seeds:intake-preview"),
            )
            .map_err(graph_error)?;
    }
    let properties = description
        .map(|d| vec![("description".into(), d.into())])
        .unwrap_or_default();
    let result = quipu::resolve_entity(
        &store,
        title.trim(),
        &properties,
        NAME_THRESHOLD,
        CANDIDATE_CAP,
    )
    .map_err(graph_error)?;
    if result.candidates.len() >= CANDIDATE_CAP {
        return Err(SdError::refused(
            "duplicate intake candidate bound reached; no create sent",
        ));
    }
    Ok(result.candidates)
}

fn graph_error(error: quipu::error::Error) -> SdError {
    SdError::failed(format!("duplicate intake resolver: {error}"))
}

// ------------------------------------------------- duplicate hints (8dda.12)

/// Hint-mode canonical-name floor: below the 0.99 refusal gate, a near-copy
/// title is reported and recorded, never refused (unless `--strict`).
pub const HINT_NAME_THRESHOLD: f64 = 0.90;
/// How many candidates a create prints and records.
pub const HINT_TOP: usize = 3;
/// Closed seeds stay hint candidates for this many days after closing.
pub const HINT_CLOSED_DAYS: i64 = 30;
/// Quipu's durable "these are not the same" edge; quipu's resolver honours
/// it in both directions, and so do seeds' hints.
pub const DISTINCT_FROM: &str = "https://quipu.dev/ontology/distinctFrom";

/// `seeds:possibleDuplicateOf`: an unresolved duplicate hint (stopgap term,
/// to be proposed for camayoc like `seeds:workflowRun`).
pub fn possible_duplicate_of() -> String {
    crate::vocab::seeds("possibleDuplicateOf")
}
/// `seeds:duplicateHintState`: `hinted`, `none` or `unknown` (the resolver
/// failed: absence of hints is not established).
pub fn hint_state() -> String {
    crate::vocab::seeds("duplicateHintState")
}
/// `seeds:duplicateHint`: JSON detail of the recorded candidates
/// (Quipu's `resolution_hints` shape: iri, score, matched_on).
pub fn hint_detail() -> String {
    crate::vocab::seeds("duplicateHint")
}

/// One duplicate candidate as printed and recorded.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hint {
    pub id: String,
    pub title: String,
    pub status: String,
    pub score: f64,
    pub matched_on: String,
}

/// Inbox pointers and delivery records are never duplicate candidates.
pub fn is_inbox_pointer(seed: &Seed) -> bool {
    crate::engine::deadline_message(seed)
}

/// Whether a seed being created is itself hinted: the hinted side gets the
/// same exclusions as the corpus, so an inbox pointer, ephemeral seed or
/// machine record (ops, ask) never gets `possibleDuplicateOf`, never shows
/// in `sd dupes` and is never refused by `--strict` for a hint.
pub fn hint_subject(
    title: &str,
    labels: &[String],
    issue_type: Option<&str>,
    ephemeral: bool,
) -> bool {
    !crate::engine::inbox_message(title, labels) && wants_directive_hint(issue_type, ephemeral)
}

/// Whether `seed` belongs to the hint corpus at `now` (epoch seconds):
/// open-ish work, or closed within [`HINT_CLOSED_DAYS`]; never inbox
/// pointers, ephemeral seeds or machine records (ops, ask job reports).
pub fn hint_eligible(seed: &Seed, now: Option<i64>) -> bool {
    if is_inbox_pointer(seed)
        || !wants_directive_hint(Some(&seed.issue_type), seed.ephemeral)
        || seed.status == "tombstone"
    {
        return false;
    }
    if seed.status != "closed" {
        return true;
    }
    match (
        now,
        seed.closed_at.as_deref().and_then(crate::engine::epoch_of),
    ) {
        (Some(now), Some(closed)) => now - closed <= HINT_CLOSED_DAYS * 86_400,
        _ => false,
    }
}

/// The `distinctFrom` partners recorded on either side of `id`.
pub fn distinct_partners<'a>(
    seeds: impl IntoIterator<Item = &'a Seed>,
    id: &str,
) -> std::collections::BTreeSet<String> {
    let me = crate::vocab::item_iri(id);
    let mut out = std::collections::BTreeSet::new();
    for seed in seeds {
        for (p, o) in &seed.extra {
            if p != DISTINCT_FROM {
                continue;
            }
            if let crate::model::Obj::Iri(target) = o {
                if seed.id == id {
                    if let Some(t) = crate::vocab::item_id(target) {
                        out.insert(t);
                    }
                } else if target == &me {
                    out.insert(seed.id.clone());
                }
            }
        }
    }
    out
}

/// Canonical-name candidates (Quipu's resolver, Jaro-Winkler included) at
/// `threshold` over exactly the supplied seeds, whatever their status.
pub fn name_candidates_at<'a>(
    seeds: impl IntoIterator<Item = &'a Seed>,
    title: &str,
    description: Option<&str>,
    threshold: f64,
) -> Result<Vec<EntityCandidate>> {
    let mut store = Store::open_in_memory().map_err(graph_error)?;
    let label = store
        .intern(crate::vocab::RDFS_LABEL)
        .map_err(graph_error)?;
    let mut facts = Vec::new();
    for seed in seeds {
        facts.push(Datum {
            entity: store
                .intern(&crate::vocab::item_iri(&seed.id))
                .map_err(graph_error)?,
            attribute: label,
            value: Value::Str(seed.title.clone()),
            valid_from: "1970-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    if facts.is_empty() {
        return Ok(vec![]);
    }
    store
        .transact(
            &facts,
            "1970-01-01T00:00:00Z",
            None,
            Some("seeds:hint-preview"),
        )
        .map_err(graph_error)?;
    let properties = description
        .map(|d| vec![("description".into(), d.into())])
        .unwrap_or_default();
    let result = quipu::resolve_entity(&store, title.trim(), &properties, threshold, CANDIDATE_CAP)
        .map_err(graph_error)?;
    Ok(result.candidates)
}

/// The facts a hint result records on the seed: the state, one
/// `possibleDuplicateOf` edge per candidate, and the JSON detail.
pub fn hint_facts(hints: &[Hint], unknown: Option<&str>) -> Vec<(String, crate::model::Obj)> {
    use crate::model::Obj;
    let state = match (unknown, hints.is_empty()) {
        (Some(_), _) => "unknown",
        (None, false) => "hinted",
        (None, true) => "none",
    };
    let mut out = vec![(hint_state(), Obj::Str(state.into()))];
    out.extend(hints.iter().map(|h| {
        (
            possible_duplicate_of(),
            Obj::Iri(crate::vocab::item_iri(&h.id)),
        )
    }));
    let detail = serde_json::json!({
        "threshold": HINT_NAME_THRESHOLD,
        "unknown": unknown,
        "candidates": hints.iter().map(|h| serde_json::json!({
            "iri": crate::vocab::item_iri(&h.id), "score": h.score, "matched_on": h.matched_on,
        })).collect::<Vec<_>>(),
    });
    out.push((hint_detail(), Obj::Str(detail.to_string())));
    out
}

/// Whether a predicate is one [`hint_facts`] writes (replaced on re-hint).
pub fn is_hint_predicate(p: &str) -> bool {
    p == hint_state() || p == possible_duplicate_of() || p == hint_detail()
}

/// Where a hint lookup ran, which decides what an UNKNOWN line says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintContext {
    /// `sd create`: the remote half of the refusal gate was skipped too.
    Create,
    /// `sd update` re-hint: there is no gate, and the edit is saved.
    Edit,
}

/// The stderr lines for a hint result. UNKNOWN is never "no duplicates".
pub fn hint_lines(hints: &[Hint], unknown: Option<&str>, at: HintContext) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(why) = unknown {
        lines.push(match at {
            HintContext::Create => format!(
                "duplicate check UNKNOWN ({why}): the embedding half of the refusal gate was not applied, only the local name gate, and no duplicate hints were found; the create proceeded, run `sd dupes` later or use --strict to refuse"
            ),
            HintContext::Edit => format!(
                "duplicate hints UNKNOWN ({why}); the edit is saved and its old hints are cleared, run `sd dupes` later"
            ),
        });
    }
    if !hints.is_empty() {
        lines.push(
            "possible duplicates (comment there instead, or `sd dupes resolve <new> <id> --distinct`):"
                .into(),
        );
        lines.extend(
            hints
                .iter()
                .map(|h| format!("  {} [{}] {:.2} {}", h.id, h.status, h.score, h.title)),
        );
    }
    lines
}

/// An open hint pair for grooming: `seed` may duplicate `of`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HintPair {
    pub id: String,
    pub title: String,
    pub status: String,
    pub duplicate_of: String,
    pub duplicate_of_title: Option<String>,
    pub duplicate_of_status: Option<String>,
}

/// Every unresolved hint: the hinted seed is not closed and the pair has no
/// `distinctFrom` on either side. One pass collects every `distinctFrom`
/// pair, so this is linear in the facts, not quadratic in the seeds.
pub fn open_hint_pairs(snap: &crate::model::Snapshot) -> Vec<HintPair> {
    let pred = possible_duplicate_of();
    let mut distinct = std::collections::BTreeSet::new();
    for seed in snap.seeds.values() {
        for (p, o) in &seed.extra {
            if let (true, crate::model::Obj::Iri(target)) = (p == DISTINCT_FROM, o) {
                if let Some(t) = crate::vocab::item_id(target) {
                    distinct.insert((seed.id.clone(), t.clone()));
                    distinct.insert((t, seed.id.clone()));
                }
            }
        }
    }
    let mut out = Vec::new();
    for seed in snap.seeds.values() {
        // Only live, hint-eligible work is a hinted side: a hint recorded on
        // an inbox pointer, ephemeral seed or machine record is not listed.
        if seed.status == "closed" || !hint_eligible(seed, None) {
            continue;
        }
        for (p, o) in &seed.extra {
            let (true, crate::model::Obj::Iri(target)) = (p == &pred, o) else {
                continue;
            };
            let Some(of) = crate::vocab::item_id(target) else {
                continue;
            };
            if distinct.contains(&(seed.id.clone(), of.clone())) {
                continue;
            }
            let other = snap.seeds.get(&of);
            out.push(HintPair {
                id: seed.id.clone(),
                title: seed.title.clone(),
                status: seed.status.clone(),
                duplicate_of: of,
                duplicate_of_title: other.map(|s| s.title.clone()),
                duplicate_of_status: other.map(|s| s.status.clone()),
            });
        }
    }
    out
}

/// How many directive hints a new seed gets (aegis-8dda.9).
pub const DIRECTIVE_HINTS: usize = 3;
/// The name prefix of operator Directives (the operator's own words), as
/// opposed to crew rulings also typed Directive. Replace with the governed
/// directiveKind once it is accepted (Quipu proposal 61).
pub const OPERATOR_DIRECTIVE_PREFIX: &str = "stiwi-";

/// Whether a new seed already names the Directive it serves: a line that
/// starts `Directive:` (as item templates write it), or an operator
/// directive's name (`stiwi-directive-...`, `stiwi-...`) anywhere in the text.
/// Advisory only: this decides whether to print hints, never whether to create.
pub fn names_directive(title: &str, description: Option<&str>) -> bool {
    let text = format!("{title}\n{}", description.unwrap_or_default()).to_lowercase();
    text.lines().any(|line| {
        line.trim_start()
            .trim_start_matches(['-', '*', ' '])
            .starts_with("directive:")
    }) || text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|word| {
            word.starts_with(OPERATOR_DIRECTIVE_PREFIX)
                && word.len() > OPERATOR_DIRECTIVE_PREFIX.len()
        })
}

/// Machine records (ops, ask) and ephemeral seeds are exempt: they serve
/// the work they record, not a directive of their own.
pub fn wants_directive_hint(issue_type: Option<&str>, ephemeral: bool) -> bool {
    !ephemeral && !matches!(issue_type.unwrap_or("task"), "ops" | "ask")
}

/// The stderr lines for a seed that names no directive. A failed lookup is
/// reported as UNKNOWN and never blocks the create (sattler 2026-11-24).
pub fn directive_hint_lines(hints: Result<Option<Vec<(String, f64)>>>) -> Vec<String> {
    match hints {
        Ok(None) => vec![],
        Ok(Some(found)) if found.is_empty() => vec![
            "this seed names no directive, and none is near it; if it serves one, add a 'Directive: <name>' line".into(),
        ],
        Ok(Some(found)) => {
            let mut lines = vec![
                "this seed names no directive. Nearest operator directives (add a 'Directive: <name>' line if one applies):".into(),
            ];
            lines.extend(found.iter().take(DIRECTIVE_HINTS).map(|(iri, score)| {
                format!("  {} ({score:.2})", iri.rsplit('/').next().unwrap_or(iri))
            }));
            lines
        }
        Err(error) => vec![format!(
            "this seed names no directive; directive hints UNKNOWN ({}); the create was not affected",
            error.message
        )],
    }
}

#[cfg(test)]
mod directive_hint_tests {
    use super::*;

    #[test]
    fn a_named_directive_suppresses_the_hint() {
        assert!(names_directive(
            "x",
            Some("Directive: stiwi-directive-backlog-cap-200")
        ));
        assert!(names_directive(
            "x",
            Some("  - directive: anything the item names")
        ));
        assert!(names_directive(
            "fix it (stiwi-directive-quipu-search-2026-11-01)",
            None
        ));
        assert!(!names_directive(
            "Bump the tautulli image",
            Some("CVE fix; directive lookup later")
        ));
        assert!(!names_directive(
            "stiwi asked for this",
            Some("stiwi- is not a name")
        ));
    }

    #[test]
    fn machine_records_and_ephemeral_seeds_are_exempt() {
        assert!(wants_directive_hint(None, false));
        assert!(wants_directive_hint(Some("bug"), false));
        assert!(!wants_directive_hint(Some("ops"), false));
        assert!(!wants_directive_hint(Some("ask"), false));
        assert!(!wants_directive_hint(Some("task"), true));
    }

    #[test]
    fn hints_never_fail_and_say_unknown_when_the_lookup_fails() {
        assert!(directive_hint_lines(Ok(None)).is_empty());
        let none = directive_hint_lines(Ok(Some(vec![])));
        assert_eq!(none.len(), 1);
        let found = directive_hint_lines(Ok(Some(vec![
            ("http://aegis.gastown.local/ontology/stiwi-a".into(), 0.58),
            ("http://aegis.gastown.local/ontology/stiwi-b".into(), 0.34),
            ("http://aegis.gastown.local/ontology/stiwi-c".into(), 0.27),
            ("http://aegis.gastown.local/ontology/stiwi-d".into(), 0.20),
        ])));
        assert_eq!(found.len(), 1 + DIRECTIVE_HINTS);
        assert_eq!(found[1], "  stiwi-a (0.58)");
        let failed = directive_hint_lines(Err(SdError::refused("timeout")));
        assert_eq!(failed.len(), 1);
        assert!(failed[0].contains("UNKNOWN") && failed[0].contains("not affected"));
    }
}
