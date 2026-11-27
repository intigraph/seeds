//! Restrict projection queries to the two complete admitted graphs.
//! Parse algebra, including nested EXISTS; string matching is not a scope proof.
use super::*;
use spargebra::{
    algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression},
    term::NamedNodePattern,
    Query, SparqlParser,
};

fn expr(e: &Expression, graphs: &[String], scoped: bool, depth: usize) -> bool {
    use Expression::*;
    if depth > 64 {
        return false;
    }
    let next = |e| expr(e, graphs, scoped, depth + 1);
    match e {
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => true,
        Or(a, b)
        | And(a, b)
        | Equal(a, b)
        | SameTerm(a, b)
        | Greater(a, b)
        | GreaterOrEqual(a, b)
        | Less(a, b)
        | LessOrEqual(a, b)
        | Add(a, b)
        | Subtract(a, b)
        | Multiply(a, b)
        | Divide(a, b) => next(a) && next(b),
        In(a, rest) => next(a) && rest.iter().all(next),
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => next(a),
        Exists(p) => pattern(p, graphs, scoped, depth + 1),
        If(a, b, c) => next(a) && next(b) && next(c),
        Coalesce(args) | FunctionCall(_, args) => args.iter().all(next),
    }
}
fn pattern(p: &GraphPattern, graphs: &[String], scoped: bool, depth: usize) -> bool {
    use GraphPattern::*;
    if depth > 64 {
        return false;
    }
    let pat = |p| pattern(p, graphs, scoped, depth + 1);
    let ex = |e| expr(e, graphs, scoped, depth + 1);
    match p {
        Bgp { patterns } => scoped || patterns.is_empty(),
        Path { .. } => scoped,
        Graph {
            name: NamedNodePattern::NamedNode(name),
            inner,
        } => graphs.iter().any(|g| g == name.as_str()) && pattern(inner, graphs, true, depth + 1),
        Graph { .. } | Service { .. } | Lateral { .. } => false,
        Join { left, right } | Union { left, right } | Minus { left, right } => {
            pat(left) && pat(right)
        }
        LeftJoin {
            left,
            right,
            expression,
        } => pat(left) && pat(right) && expression.as_ref().is_none_or(ex),
        Filter { expr, inner } => pat(inner) && ex(expr),
        Extend {
            inner, expression, ..
        } => pat(inner) && ex(expression),
        Values { .. } => true,
        OrderBy { inner, expression } => {
            pat(inner)
                && expression.iter().all(|o| match o {
                    OrderExpression::Asc(e) | OrderExpression::Desc(e) => ex(e),
                })
        }
        Project { inner, .. } | Distinct { inner } | Reduced { inner } | Slice { inner, .. } => {
            pat(inner)
        }
        Group {
            inner, aggregates, ..
        } => {
            pat(inner)
                && aggregates.iter().all(|(_, a)| match a {
                    AggregateExpression::CountSolutions { .. } => true,
                    AggregateExpression::FunctionCall { expr, .. } => ex(expr),
                })
        }
    }
}

/// Reads that kept the temporal path because the head moved past the cut.
pub(super) static AS_OF_FALLBACKS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// True only when the store head is exactly the served cut.
pub(super) fn at_head(head: i64, cut: i64) -> bool {
    head == cut
}

pub(super) fn evaluate(
    backend: &crate::quipu_backend::QuipuBackend,
    graph: &str,
    tx: u64,
    text: &str,
) -> Result<Value> {
    if text.len() > 60 * 1024 {
        return Err(unknown("query byte budget exceeded"));
    }
    let parsed = SparqlParser::new()
        .parse_query(text)
        .map_err(|_| unknown("malformed projection query"))?;
    let p = match &parsed {
        Query::Select {
            dataset: None,
            base_iri: None,
            pattern,
        }
        | Query::Ask {
            dataset: None,
            base_iri: None,
            pattern,
        } => pattern,
        _ => {
            return Err(unknown(
                "projection requires SELECT/ASK without dataset or BASE override",
            ))
        }
    };
    let graphs = [graph.to_string(), crate::vocab::ephemeral_graph(graph)];
    if !pattern(p, &graphs, false, 0) {
        return Err(unknown("query exceeds complete projection scope"));
    }
    let cut = i64::try_from(tx).map_err(|_| unknown("local cut out of range"))?;
    // aegis-az3d.2: under as_of_tx, quipu 0.16.6 drops its idx_vaet hint and a
    // plain literal object scans the whole graph (Phase 0: 119 ms -> 0.1 ms one
    // triple, 2408 -> 276 ms heavy, at 121k items). The cut is the committed
    // head here, so the current view answers the same question. That is
    // ENFORCED, not assumed: only when the store head still equals the served
    // cut do we read current; on any mismatch we keep the exact temporal read
    // (slow but correct) and count it, so a second writer costs speed, never
    // correctness.
    let as_of_tx = if at_head(backend.store().transaction_head()?, cut) {
        None
    } else {
        AS_OF_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(cut)
    };
    evaluate_at(backend, text, as_of_tx)
}

/// The scoped read with an explicit temporal choice; `evaluate` decides it.
fn evaluate_at(
    backend: &crate::quipu_backend::QuipuBackend,
    text: &str,
    as_of_tx: Option<i64>,
) -> Result<Value> {
    let ctx = quipu::sparql::TemporalContext {
        as_of_tx,
        deadline: Some(quipu::time::Deadline::after_millis(
            super::QUERY_DEADLINE_MS,
        )),
        row_cap: Some(10_000),
        ..Default::default()
    };
    let store = backend.store();
    match quipu::sparql::query_temporal(store, text, &ctx)? {
        quipu::sparql::QueryResult::Ask(value) => Ok(json!({"result": value})),
        quipu::sparql::QueryResult::Select { variables, rows } => {
            if rows.len() > 10_000 {
                return Err(unknown("projection row budget exceeded"));
            }
            let rows: Vec<Value> = rows
                .iter()
                .map(|row| {
                    Value::Object(
                        row.iter()
                            .map(|(k, v)| (k.clone(), quipu::value_to_json(store, v)))
                            .collect(),
                    )
                })
                .collect();
            Ok(json!({"columns": variables,"count":rows.len(),"rows":rows,"truncated":false}))
        }
        _ => Err(unknown("unsupported projection query result")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::disallowed_methods)]
mod tests {
    //! aegis-az3d.2 controls: the current read equals the exact temporal read at
    //! head, and a moved head falls back to the temporal read and is counted.
    use super::*;
    use crate::backend::Ctx;
    use crate::engine::{self, BulkItem, CreateReq, UpdateReq};
    use crate::quipu_backend::QuipuBackend;
    use std::sync::atomic::Ordering;

    const GRAPH: &str = "https://seeds.local/project/az3d2";
    /// AS_OF_FALLBACKS is process-wide; the two counter tests must not interleave.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn ctx() -> Ctx {
        Ctx {
            now: "2026-11-26T00:00:00Z".into(),
            actor: "fixture".into(),
            prefix: "az".into(),
            claims: Default::default(),
        }
    }

    fn item(k: usize, agent: &str, ephemeral: bool) -> BulkItem {
        BulkItem {
            req: CreateReq {
                title: format!("item {k}"),
                assignee: Some(agent.into()),
                labels: vec![format!("l{}", k % 3)],
                ephemeral,
                ..Default::default()
            },
            notes: None,
            design: None,
        }
    }

    /// Live, closed-via-in_progress (retracted status rows), deferred-free, and ephemeral items.
    fn board() -> QuipuBackend {
        let mut b = QuipuBackend::writable(quipu::Store::open_in_memory().unwrap(), GRAPH).unwrap();
        b.store()
            .graph_create(&crate::vocab::ephemeral_graph(GRAPH))
            .unwrap();
        let items: Vec<BulkItem> = (0..30)
            .map(|k| item(k, ["malcolm", "wu", "ian"][k % 3], false))
            .collect();
        let (seeds, _) = engine::create_many(&mut b, &ctx(), &items, false).unwrap();
        let ids: Vec<String> = seeds.into_iter().map(|s| s.id).collect();
        let progress = UpdateReq {
            status: Some("in_progress".into()),
            ..Default::default()
        };
        engine::update(&mut b, &ctx(), &ids[..12], &progress).unwrap();
        engine::close(&mut b, &ctx(), &ids[..8], Some("done"), true).unwrap();
        let reassign = UpdateReq {
            assignee: Some("dearing".into()),
            ..Default::default()
        };
        engine::update(&mut b, &ctx(), &ids[12..16], &reassign).unwrap();
        let wisps: Vec<BulkItem> = (100..104).map(|k| item(k, "malcolm", true)).collect();
        engine::create_many(&mut b, &ctx(), &wisps, false).unwrap();
        b
    }

    fn queries() -> Vec<String> {
        let g = GRAPH;
        let e = crate::vocab::ephemeral_graph(GRAPH);
        let st = "<https://seeds.local/ontology/status>";
        let name = "<https://schema.org/name>";
        let heavy = |graph: &str, status: &str| {
            format!(
                "{{ GRAPH <{graph}> {{ ?s {st} \"{status}\" . FILTER EXISTS {{ ?s a ?t }} \
                 FILTER NOT EXISTS {{ ?s {name} ?title . FILTER(REGEX(STR(?title), \"^item 1\")) }} }} }}"
            )
        };
        vec![
            format!("SELECT ?s WHERE {{ GRAPH <{g}> {{ ?s {name} \"item 5\" }} }}"),
            format!("SELECT ?s ?p WHERE {{ GRAPH <{g}> {{ ?s ?p \"in_progress\" }} }}"),
            format!("SELECT ?s ?p WHERE {{ GRAPH <{g}> {{ ?s ?p \"closed\" }} }}"),
            format!("SELECT ?s ?p WHERE {{ GRAPH <{g}> {{ ?s ?p \"open\" }} }}"),
            format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{e}> {{ ?s ?p ?o }} }}"),
            format!(
                "SELECT ?s WHERE {{ {} UNION {} UNION {} }}",
                heavy(g, "open"),
                heavy(g, "in_progress"),
                heavy(&e, "open")
            ),
            format!("ASK {{ GRAPH <{g}> {{ ?s {name} \"item 7\" }} }}"),
        ]
    }

    fn sorted(v: Value) -> Value {
        let mut v = v;
        if let Some(rows) = v.get_mut("rows").and_then(Value::as_array_mut) {
            rows.sort_by_key(|r| r.to_string());
        }
        v
    }

    #[test]
    fn current_read_at_head_equals_the_exact_temporal_read() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let b = board();
        let head = b.store().transaction_head().unwrap();
        let before = AS_OF_FALLBACKS.load(Ordering::Relaxed);
        let mut nonempty = 0;
        for q in queries() {
            let temporal = sorted(evaluate_at(&b, &q, Some(head)).unwrap());
            let current = sorted(evaluate_at(&b, &q, None).unwrap());
            let chosen = sorted(evaluate(&b, GRAPH, u64::try_from(head).unwrap(), &q).unwrap());
            assert_eq!(temporal, current, "{q}");
            assert_eq!(temporal, chosen, "{q}");
            if temporal.get("count").and_then(Value::as_u64).unwrap_or(0) > 0
                || temporal.get("result") == Some(&Value::Bool(true))
            {
                nonempty += 1;
            }
        }
        // The controls must exercise real rows, including retracted in_progress and ephemeral.
        assert!(nonempty >= 6, "fixture answers too little: {nonempty}");
        assert_eq!(
            before,
            AS_OF_FALLBACKS.load(Ordering::Relaxed),
            "no fallback at head"
        );
    }

    #[test]
    fn a_moved_head_keeps_the_temporal_read_and_is_counted() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut b = board();
        let cut = b.store().transaction_head().unwrap();
        let q = format!(
            "SELECT ?s WHERE {{ GRAPH <{GRAPH}> {{ ?s <https://schema.org/name> \"item 999\" }} }}"
        );
        engine::create_many(&mut b, &ctx(), &[item(999, "newcomer", false)], false).unwrap();
        assert!(b.store().transaction_head().unwrap() > cut);
        let before = AS_OF_FALLBACKS.load(Ordering::Relaxed);
        let got = evaluate(&b, GRAPH, u64::try_from(cut).unwrap(), &q).unwrap();
        assert_eq!(
            Some(0),
            got.get("count").and_then(Value::as_u64),
            "served the old cut, not the newer write"
        );
        assert!(
            AS_OF_FALLBACKS.load(Ordering::Relaxed) > before,
            "fallback counted"
        );
        let now = evaluate_at(&b, &q, None).unwrap();
        assert_eq!(
            Some(1),
            now.get("count").and_then(Value::as_u64),
            "the write is visible at head"
        );
    }

    #[test]
    fn at_head_is_exact_equality() {
        assert!(at_head(7, 7));
        assert!(!at_head(8, 7));
        assert!(!at_head(6, 7));
    }
}
