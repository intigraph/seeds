//! Query equivalence on a fixed RDF cut, including irregular external facts.
#![allow(clippy::unwrap_used)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use quipu::sparql::query;
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};
use seeds::vocab;

const GRAPH: &str = "https://example.test/ready";

fn put(store: &mut Store, graph: &str, facts: &[(&str, &str, Value)]) {
    let graph = store.graph_create(graph).unwrap();
    let data = facts
        .iter()
        .map(|(subject, predicate, value)| Datum {
            entity: store.intern(subject).unwrap(),
            attribute: store.intern(predicate).unwrap(),
            value: value.clone(),
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store
        .transact_graph_batches(&[(graph, data)], "2026-01-01T00:00:00Z", None, None)
        .unwrap();
}

fn ids(store: &Store, text: &str) -> Vec<String> {
    let mut ids: Vec<_> = query(store, text)
        .unwrap()
        .rows()
        .iter()
        .map(|row| match &row["id"] {
            Value::Str(id) => id.clone(),
            other => panic!("unexpected identifier: {other:?}"),
        })
        .collect();
    ids.sort();
    ids
}

fn fixture() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let action = Value::Ref(store.intern(&vocab::schema("Action")).unwrap());
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let status = vocab::seeds("status");
    let identifier = vocab::schema("identifier");
    let blocked_on = vocab::quechua("blockedOn");
    let issue_type = vocab::seeds("issueType");
    for id in [
        "plain",
        "closed",
        "tombstone",
        "missing",
        "active",
        "mixed",
        "two",
        "ops",
        "ask",
        "foreign",
    ] {
        let subject = format!("urn:item:{id}");
        put(
            &mut store,
            GRAPH,
            &[
                (&subject, rdf_type, action.clone()),
                (&subject, &identifier, Value::Str(id.into())),
                (&subject, &status, Value::Str("open".into())),
            ],
        );
    }
    for (id, states) in [
        ("closed", vec!["closed"]),
        ("tombstone", vec!["tombstone"]),
        ("missing", vec![]),
        ("active", vec!["in_progress"]),
        ("mixed", vec!["closed", "open"]),
        ("two", vec!["closed", "tombstone"]),
        ("foreign", vec![]),
    ] {
        let blocker = format!("urn:blocker:{id}");
        let blocker_ref = Value::Ref(store.intern(&blocker).unwrap());
        put(
            &mut store,
            GRAPH,
            &[(&format!("urn:item:{id}"), &blocked_on, blocker_ref)],
        );
        for state in states {
            // Blockers need not themselves be Actions.
            put(
                &mut store,
                GRAPH,
                &[(&blocker, &status, Value::Str(state.into()))],
            );
        }
    }
    for id in ["ops", "ask"] {
        put(
            &mut store,
            GRAPH,
            &[
                (
                    &format!("urn:item:{id}"),
                    &issue_type,
                    Value::Str(id.into()),
                ),
                (
                    &format!("urn:item:{id}"),
                    &issue_type,
                    Value::Str("task".into()),
                ),
            ],
        );
    }
    // Foreign status must not block a source in the selected graph.
    put(
        &mut store,
        "urn:foreign",
        &[("urn:blocker:foreign", &status, Value::Str("open".into()))],
    );
    store
}

#[test]
fn correlated_blockers_preserve_ready_membership_and_multiplicity() {
    // Separate connections and caches: the reference cannot warm the candidate.
    let store = fixture();
    let old_store = fixture();
    let old = format!(
        "PREFIX schema: <{}> PREFIX quechua: <{}> PREFIX seeds: <{}>
         SELECT ?id WHERE {{ GRAPH <{GRAPH}> {{
           ?item seeds:status \"open\" .
           ?item a schema:Action ; schema:identifier ?id .
           FILTER NOT EXISTS {{ ?item seeds:issueType \"ops\" }}
           FILTER NOT EXISTS {{ ?item seeds:issueType \"ask\" }}
           FILTER NOT EXISTS {{ ?item quechua:blockedOn ?blocker .
             ?blocker seeds:status ?bs . ?item a schema:Action .
             FILTER(?bs != \"closed\" && ?bs != \"tombstone\") }}
         }} }}",
        vocab::SCHEMA,
        vocab::QUECHUA,
        vocab::SEEDS
    );
    let expected = vec!["closed", "foreign", "missing", "plain", "tombstone", "two"];
    let candidate = format!("{} LIMIT 10000", vocab::ready_query(GRAPH));
    let old = format!("{old} LIMIT 10000");
    #[cfg(not(target_arch = "wasm32"))]
    store.enable_statement_profile();
    #[cfg(not(target_arch = "wasm32"))]
    old_store.enable_statement_profile();
    #[cfg(not(target_arch = "wasm32"))]
    let candidate_span = quipu::statement_profile::Span::arm();
    assert_eq!(ids(&store, &candidate), expected);
    #[cfg(not(target_arch = "wasm32"))]
    let candidate_profile = candidate_span.finish().unwrap();
    #[cfg(not(target_arch = "wasm32"))]
    let old_span = quipu::statement_profile::Span::arm();
    assert_eq!(ids(&old_store, &old), expected);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let old_profile = old_span.finish().unwrap();
        let counts = |profile: &quipu::statement_profile::Totals| -> u64 {
            profile
                .by_sql
                .iter()
                .filter(|(sql, _)| {
                    sql.starts_with(
                        "SELECT COUNT(*) FROM facts WHERE op = 1 AND valid_to IS NULL AND g =",
                    )
                })
                .map(|(_, (count, _))| count)
                .sum()
        };
        assert!(
            counts(&candidate_profile) <= 1,
            "ready must not count the entire graph once per candidate"
        );
        assert!(
            counts(&old_profile) > 1,
            "old query must exercise repeated per-row affordability counts"
        );
    }
}
