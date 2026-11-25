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
