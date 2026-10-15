//! Completion conditions read from the same Quipu store, never fetched by IRI.
use super::{Blocker, State};
use crate::backend::Backend;
use crate::error::{Result, SdError};
use crate::vocab;

/// Validate before placing an IRI into generated SPARQL.
pub fn validate_iri(value: &str) -> Result<()> {
    oxiri::Iri::parse(value)
        .map(|_| ())
        .map_err(|_| SdError::usage("blocker and graph must be absolute IRIs"))
}

/// Observe an ASK. Missing/invalid/error responses remain UNKNOWN, not false.
pub fn observe(backend: &dyn Backend, blocker: &Blocker, at: Option<u64>) -> State {
    let Ok(response) = backend.blocker_query(&blocker.ask, blocker.graph.as_deref(), at) else {
        return State::Unknown;
    };
    if response.get("error").is_some()
        || response
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return State::Unknown;
    }
    match response
        .get("boolean")
        .or_else(|| response.get("result"))
        .and_then(serde_json::Value::as_bool)
    {
        Some(true) => State::Done,
        Some(false) => State::Pending,
        None => State::Unknown,
    }
}

/// An exact-name lookup; ambiguous or incomplete results refuse resolution.
pub fn resolve_name(backend: &dyn Backend, name: &str, graph: Option<&str>) -> Result<String> {
    if validate_iri(name).is_ok() {
        return Ok(name.into());
    }
    let text = crate::validate::escape_literal(name);
    let pattern = format!("?node <http://www.w3.org/2000/01/rdf-schema#label> \"{text}\"");
    let query = if graph.is_some() {
        format!("SELECT DISTINCT ?node WHERE {{ {pattern} }} LIMIT 2")
    } else {
        format!("SELECT DISTINCT ?node WHERE {{ {{ {pattern} }} UNION {{ GRAPH ?g {{ {pattern} }} }} }} LIMIT 2")
    };
    let response = backend.blocker_query(&query, graph, None)?;
    if response
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Err(SdError::refused("blocker name lookup is incomplete"));
    }
    let rows = response["results"]["bindings"]
        .as_array()
        .ok_or_else(|| SdError::refused("blocker name lookup is unreadable"))?;
    if rows.len() != 1 {
        return Err(SdError::usage(
            "blocker name is absent or ambiguous; use its exact IRI and --blocker-graph",
        ));
    }
    let node = &rows[0]["node"];
    if node["type"] != "uri" {
        return Err(SdError::refused("blocker name did not resolve to an IRI"));
    }
    let iri = node["value"]
        .as_str()
        .ok_or_else(|| SdError::refused("blocker name binding is missing"))?;
    validate_iri(iri)?;
    Ok(iri.into())
}

/// Resolve a default from an exact governed type. Unknown types require an
/// explicit condition; suffix matching and namespace guessing are not used.
pub fn default_kind(backend: &dyn Backend, iri: &str, graph: Option<&str>) -> Result<String> {
    validate_iri(iri)?;
    let query = format!("SELECT ?type WHERE {{ <{iri}> a ?type }} LIMIT 64");
    let response = backend.blocker_query(&query, graph, None)?;
    let rows = response["results"]["bindings"]
        .as_array()
        .ok_or_else(|| SdError::refused("blocker typing is unreadable"))?;
    if rows.len() >= 64
        || response
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return Err(SdError::refused("blocker typing is incomplete"));
    }
    let mut kinds = std::collections::BTreeSet::new();
    for row in rows {
        if row["type"]["type"] != "uri" {
            return Err(SdError::refused("blocker type is not an IRI"));
        }
        let Some(ty) = row["type"]["value"].as_str() else {
            return Err(SdError::refused("blocker type binding missing"));
        };
        let candidates = [
            (vocab::schema("Action"), "closed"),
            (format!("{}WorkItem", vocab::AEGIS), "closed"),
            (format!("{}PullRequest", vocab::AEGIS), "merged"),
            (format!("{}GitPullRequest", vocab::AEGIS), "merged"),
            (format!("{}Release", vocab::AEGIS), "published"),
            (format!("{}GitRelease", vocab::AEGIS), "published"),
            (format!("{}Directive", vocab::AEGIS), "exists"),
            (format!("{}Decision", vocab::AEGIS), "decided"),
            (format!("{}DecisionRecord", vocab::AEGIS), "decided"),
        ];
        for (class, kind) in candidates {
            if ty == class {
                kinds.insert(kind.to_string());
            }
        }
    }
    if kinds.len() != 1 {
        return Err(SdError::usage(
            "no unambiguous default for blocker type; declare --done-when or --ask and its graph",
        ));
    }
    kinds
        .into_iter()
        .next()
        .ok_or_else(|| SdError::failed("condition missing"))
}

/// Discover a target's unique graph using exact-IRI lookups. Ambiguity requires
/// the author to declare the dataset instead of combining unrelated states.
pub fn discover_graph(backend: &dyn Backend, iri: &str) -> Result<Option<String>> {
    validate_iri(iri)?;
    let root = backend.blocker_query(&format!("ASK {{ <{iri}> ?p ?o }}"), None, None)?;
    let root_exists = root
        .get("boolean")
        .or_else(|| root.get("result"))
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| SdError::refused("ROOT blocker discovery is unreadable"))?;
    let result = backend.blocker_query(
        &format!("SELECT DISTINCT ?graph WHERE {{ GRAPH ?graph {{ <{iri}> ?p ?o }} }} LIMIT 2"),
        None,
        None,
    )?;
    let rows = result["results"]["bindings"]
        .as_array()
        .ok_or_else(|| SdError::refused("named blocker discovery is unreadable"))?;
    if rows.len() > 1 || (root_exists && !rows.is_empty()) || result["truncated"] == true {
        return Err(SdError::refused(
            "blocker occurs in multiple graphs; declare --blocker-graph",
        ));
    }
    match rows.first() {
        None => Ok(None),
        Some(row) => {
            if row["graph"]["type"] != "uri" {
                return Err(SdError::refused("blocker graph is not an IRI"));
            }
            let graph = row["graph"]["value"]
                .as_str()
                .ok_or_else(|| SdError::refused("blocker graph missing"))?;
            validate_iri(graph)?;
            Ok(Some(graph.into()))
        }
    }
}

/// Generate the declared read condition; a closed unmerged PR is not merged.
pub fn ask_for(iri: &str, kind: &str) -> Result<String> {
    validate_iri(iri)?;
    let status = format!("{}status", vocab::AEGIS);
    let body = match kind {
        "closed" => format!("{{ <{iri}> <{}> \"closed\" }} UNION {{ <{iri}> <{status}> \"closed\" }}", vocab::term::status()),
        "merged" => format!("{{ <{iri}> <{status}> \"merged\" }} UNION {{ <{iri}> <{}merged> true }} UNION {{ <{iri}> <{}mergedAt> ?at }}", vocab::AEGIS, vocab::AEGIS),
        "published" => format!("{{ <{iri}> <{status}> \"published\" }} UNION {{ <{iri}> <{}> ?at }}", vocab::schema("datePublished")),
        "exists" => format!("<{iri}> ?predicate ?value"),
        "decided" => format!("{{ <{iri}> <{status}> \"decided\" }} UNION {{ <{iri}> <{}decision> ?decision }}", vocab::AEGIS),
        _ => return Err(SdError::usage("unknown done condition; use closed, merged, published, exists, decided or --ask")),
    };
    Ok(format!("ASK {{ {body} }}"))
}
