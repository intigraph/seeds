//! Discovery semantics at board scale, including externally written defaults.
use super::*;
use crate::backend::SeedQuery;

#[test]
fn owner_open_family_starts_with_status_and_closed_retains_owner_first() {
    for status in [
        "open",
        "in_progress",
        "blocked",
        "hooked",
        "deferred",
        "closed",
    ] {
        let query = SeedQuery {
            assignee: Some("reader".into()),
            statuses: Some([status.into()].into()),
            ..Default::default()
        };
        let pattern = seed_query_pattern_var_with_status_defaults(&query, "?s", false).unwrap();
        let status_pos = pattern.find(&format!("<{}>", term::status())).unwrap();
        let owner_pos = pattern.find(&format!("<{}>", term::assigned_to())).unwrap();
        assert_eq!(status_pos < owner_pos, status != "closed", "{pattern}");
    }
}

#[test]
fn indexed_owner_discovery_keeps_board_ids_and_default_open_semantics() {
    use quipu::{Datum, Op, Store, Value};
    let mut store = Store::open_in_memory().unwrap();
    let graph = store.graph_create("urn:board").unwrap();
    let ty = store.intern(RDF_TYPE).unwrap();
    let class = store.intern(&term::work_item()).unwrap();
    let owner = store.intern(&term::assigned_to()).unwrap();
    let reader = store.intern(&principal_iri("reader")).unwrap();
    let status = store.intern(&term::status()).unwrap();
    let datum = |entity, attribute, value| Datum {
        entity,
        attribute,
        value,
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
        op: Op::Assert,
    };
    let mut datums = Vec::new();
    for i in 0..4000 {
        let entity = store.intern(&format!("urn:seed:{i:04}")).unwrap();
        for (attribute, value) in [
            (ty, Value::Ref(class)),
            (
                owner,
                if i == 0 {
                    Value::Str("reader".into())
                } else {
                    Value::Ref(reader)
                },
            ),
            (
                status,
                Value::Str(if i < 50 { "open" } else { "closed" }.into()),
            ),
        ] {
            datums.push(datum(entity, attribute, value));
        }
    }
    store
        .transact_to_graph(&datums, "2026-01-01T00:00:00Z", None, None, graph)
        .unwrap();
    let query = SeedQuery {
        assignee: Some("reader".into()),
        statuses: Some(["open".into()].into()),
        ..Default::default()
    };
    let ids = |store: &Store, include_default| {
        let pattern =
            seed_query_pattern_var_with_status_defaults(&query, "?s", include_default).unwrap();
        let q = format!("SELECT ?s WHERE {{ GRAPH <urn:board> {{ {pattern} }} }}");
        let quipu::sparql::QueryResult::Select { rows, .. } =
            quipu::sparql::query(store, &q).unwrap()
        else {
            panic!("SELECT required")
        };
        rows.into_iter()
            .map(|row| format!("{:?}", row["s"]))
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(ids(&store, false).len(), 50);
    // The conservative fallback is deliberately expensive on a large board.
    // Keep the large-board fast path above; narrow the current view before
    // comparing default semantics, preserving all historical facts.
    let retractions = datums
        .iter()
        .skip(50 * 3)
        .cloned()
        .map(|mut datum| {
            datum.op = Op::Retract;
            datum
        })
        .collect::<Vec<_>>();
    store
        .transact_to_graph(&retractions, "2026-01-01T00:00:01Z", None, None, graph)
        .unwrap();
    assert_eq!(ids(&store, false), ids(&store, true));
    for (i, value) in [
        (4000, None),
        (4001, Some(Value::Int(7))),
        (4002, Some(Value::Ref(reader))),
    ] {
        let entity = store.intern(&format!("urn:seed:{i:04}")).unwrap();
        let mut facts = vec![
            (ty, Value::Ref(class)),
            (owner, Value::Str("reader".into())),
        ];
        if let Some(v) = value {
            facts.push((status, v));
        }
        let facts = facts
            .into_iter()
            .map(|(attribute, value)| datum(entity, attribute, value))
            .collect::<Vec<_>>();
        store
            .transact_to_graph(&facts, "2026-01-01T00:00:00Z", None, None, graph)
            .unwrap();
    }
    assert_eq!(ids(&store, true).len(), 53);
}
