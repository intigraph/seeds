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
    let ctx = quipu::sparql::TemporalContext {
        as_of_tx: Some(i64::try_from(tx).map_err(|_| unknown("local cut out of range"))?),
        deadline: Some(quipu::time::Deadline::after_millis(1000)),
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
