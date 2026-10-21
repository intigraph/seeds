//! Owner-list query planning without changing decoded default semantics.
use super::*;

impl RemoteBackend {
    pub(super) fn owner_seed_query_pattern(
        &self,
        graph: &str,
        query: &crate::backend::SeedQuery,
    ) -> Result<Option<String>> {
        let needs_default =
            query.assignee.is_some() && query.statuses.as_ref().is_some_and(|s| s.contains("open"));
        let include_default = !needs_default || self.may_have_status_defaults(graph)?;
        Ok(vocab::seed_query_pattern_var_with_status_defaults(
            query,
            "?s",
            include_default,
        ))
    }

    fn may_have_status_defaults(&self, graph: &str) -> Result<bool> {
        if let Some(present) = self.status_defaults.borrow().get(graph) {
            return Ok(*present);
        }
        // A negative canonical-status check proves no item needs a default.
        // Foreign plain strings may conservatively keep the old branch on;
        // the existing decoder still decides exact membership. No snapshot,
        // count, or seed-body hydration, and no cached error-as-absence.
        let query = status_default_query(graph);
        let answer = self.post(
            "/query",
            "application/json",
            &json!({"query": query}).to_string(),
            false,
        )?;
        let present = serde_json::from_str::<Json>(&answer)
            .ok()
            .and_then(|v| v.get("boolean").and_then(Json::as_bool))
            .ok_or_else(|| {
                SdError::failed("status-default existence check did not return a boolean")
            })?;
        self.status_defaults
            .borrow_mut()
            .insert(graph.to_string(), present);
        Ok(present)
    }
}

fn status_default_query(graph: &str) -> String {
    let statuses = crate::model::STATUSES
        .iter()
        .map(|status| format!("\"{}\"", escape_literal(status)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "ASK {{ GRAPH <{graph}> {{ ?s a <{}> . OPTIONAL {{ ?s <{}> ?status \
         VALUES ?status {{ {statuses} }} }} FILTER(isIRI(?s) && !BOUND(?status)) }} }}",
        term::work_item(),
        term::status(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_probe_matches_regular_status_defaults_without_unioning_graphs() {
        use quipu::{Datum, Store, Value};
        let mut store = Store::open_in_memory().unwrap();
        let graph = store.graph_create("urn:status-test").unwrap();
        let ty = store.intern(vocab::RDF_TYPE).unwrap();
        let class = store.intern(&term::work_item()).unwrap();
        let status = store.intern(&term::status()).unwrap();
        let entity = store.intern("urn:seed:one").unwrap();
        let datum = |attribute, value| Datum {
            entity,
            attribute,
            value,
            valid_from: "2026-01-01T00:00:00Z".into(),
            valid_to: None,
            op: quipu::Op::Assert,
        };
        let probe = |store: &Store| {
            let quipu::sparql::QueryResult::Ask(missing) =
                quipu::sparql::query(store, &status_default_query("urn:status-test")).unwrap()
            else {
                panic!("ASK required")
            };
            missing
        };
        assert!(!probe(&store), "empty project has no defaulted item");
        store
            .transact_to_graph(
                &[datum(ty, Value::Ref(class))],
                "2026-01-01T00:00:00Z",
                None,
                None,
                graph,
            )
            .unwrap();
        assert!(probe(&store), "typed item missing status is open");
        store
            .transact_to_graph(
                &[datum(status, Value::Int(7))],
                "2026-01-01T00:00:00Z",
                None,
                None,
                graph,
            )
            .unwrap();
        assert!(probe(&store), "irregular status is open");
        store
            .transact_to_graph(
                &[datum(status, Value::Str("closed".into()))],
                "2026-01-01T00:00:00Z",
                None,
                None,
                graph,
            )
            .unwrap();
        assert!(
            !probe(&store),
            "one regular status prevents default despite irregular extras"
        );
        // Literal value equality must not mistake language/datatype variants
        // of a canonical spelling for a canonical plain-string status.
        let other = store.graph_create("urn:other").unwrap();
        store
            .transact_to_graph(
                &[datum(ty, Value::Ref(class))],
                "2026-01-01T00:00:00Z",
                None,
                None,
                other,
            )
            .unwrap();
        assert!(
            !probe(&store),
            "other graph defaults do not contaminate this graph"
        );
        for (index, value) in [
            Value::Lang {
                lexical: "closed".into(),
                lang: "en".into(),
            },
            Value::Typed {
                lexical: "closed".into(),
                datatype: "http://www.w3.org/2001/XMLSchema#normalizedString".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            let subject = store.intern(&format!("urn:seed:typed:{index}")).unwrap();
            let facts = [(ty, Value::Ref(class)), (status, value)]
                .into_iter()
                .map(|(attribute, value)| Datum {
                    entity: subject,
                    attribute,
                    value,
                    valid_from: "2026-01-01T00:00:00Z".into(),
                    valid_to: None,
                    op: quipu::Op::Assert,
                })
                .collect::<Vec<_>>();
            store
                .transact_to_graph(&facts, "2026-01-01T00:00:00Z", None, None, graph)
                .unwrap();
            assert!(
                probe(&store),
                "noncanonical typed/language status must keep fallback enabled"
            );
            let cleanup = facts
                .iter()
                .cloned()
                .map(|mut datum| {
                    datum.op = quipu::Op::Retract;
                    datum
                })
                .collect::<Vec<_>>();
            store
                .transact_to_graph(&cleanup, "2026-01-01T00:00:01Z", None, None, graph)
                .unwrap();
            assert!(!probe(&store), "each typed case must stand alone");
        }
    }
    #[test]
    fn presence_probe_covers_a_large_board_and_detects_a_late_missing_status() {
        use quipu::{Datum, Op, Store, Value};
        let mut store = Store::open_in_memory().unwrap();
        let graph = store.graph_create("urn:large-board").unwrap();
        let ty = store.intern(vocab::RDF_TYPE).unwrap();
        let class = store.intern(&term::work_item()).unwrap();
        let status = store.intern(&term::status()).unwrap();
        let mut datums = Vec::new();
        for i in 0..4000 {
            let entity = store.intern(&format!("urn:large-item:{i:04}")).unwrap();
            for (attribute, value) in [
                (ty, Value::Ref(class)),
                (status, Value::Str("closed".into())),
            ] {
                datums.push(Datum {
                    entity,
                    attribute,
                    value,
                    valid_from: "2026-01-01T00:00:00Z".into(),
                    valid_to: None,
                    op: Op::Assert,
                });
            }
        }
        store
            .transact_to_graph(&datums, "2026-01-01T00:00:00Z", None, None, graph)
            .unwrap();
        let probe = status_default_query("urn:large-board");
        assert!(matches!(
            quipu::sparql::query(&store, &probe).unwrap(),
            quipu::sparql::QueryResult::Ask(false)
        ));
        let entity = store.intern("urn:large-item:4000").unwrap();
        store
            .transact_to_graph(
                &[Datum {
                    entity,
                    attribute: ty,
                    value: Value::Ref(class),
                    valid_from: "2026-01-01T00:00:01Z".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-01-01T00:00:01Z",
                None,
                None,
                graph,
            )
            .unwrap();
        assert!(matches!(
            quipu::sparql::query(&store, &probe).unwrap(),
            quipu::sparql::QueryResult::Ask(true)
        ));
    }
}
