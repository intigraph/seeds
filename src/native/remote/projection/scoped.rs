//! Positive, typed compatibility proof. Root discovery includes candidates
//! without status, and ID owners are checked outside the selected filters.
use super::*;

type Rows = Vec<BTreeMap<String, Obj>>;
type Facts = BTreeMap<String, BTreeMap<String, Vec<Obj>>>;

fn iri(value: &str) -> Result<String> {
    if !value.contains(':')
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>\"{}|^`\\".contains(c))
    {
        return Err(invalid());
    }
    Ok(format!("<{value}>"))
}

fn predicate(name: &str) -> String {
    match name {
        "priority" => term::priority(),
        "type" => term::issue_type(),
        _ => term::assigned_to(),
    }
}

impl RemoteBackend {
    // Unlike select_marked, reject malformed terms/bindings instead of silently
    // omitting them. W3C results omit truncated: a full LIMIT is also incomplete.
    fn projection_rows(&self, query: &str, vars: &[&str]) -> Result<Option<Rows>> {
        let text = self.post(
            "/query",
            "application/json",
            &json!({"query":query}).to_string(),
            false,
        )?;
        let value: Json = serde_json::from_str(&text).map_err(|_| invalid())?;
        if let Some(flag) = value.get("truncated") {
            if flag.as_bool().ok_or_else(invalid)? {
                return Ok(None);
            }
        }
        let rows = value["results"]["bindings"]
            .as_array()
            .ok_or_else(invalid)?;
        if rows.len() >= ROW_CAP {
            return Ok(None);
        }
        let mut out = Vec::new();
        for row in rows {
            let object = row.as_object().ok_or_else(invalid)?;
            if object.len() != vars.len() {
                return Err(invalid());
            }
            let mut parsed = BTreeMap::new();
            for &name in vars {
                let binding = object.get(name).ok_or_else(invalid)?;
                if !binding.is_object()
                    || binding["value"].as_str().is_none()
                    || binding.get("datatype").is_some() && binding.get("xml:lang").is_some()
                    || binding["type"] == "uri"
                        && (binding.get("datatype").is_some() || binding.get("xml:lang").is_some())
                {
                    return Err(invalid());
                }
                if binding.get("datatype").is_some_and(|v| !v.is_string())
                    || binding.get("xml:lang").is_some_and(|v| !v.is_string())
                {
                    return Err(invalid());
                }
                parsed.insert(name.into(), term(binding).ok_or_else(invalid)?);
            }
            out.push(parsed);
        }
        Ok(Some(out))
    }

    pub(super) fn projection_scope(
        &self,
        req: &CountReq,
        required: &BTreeSet<&str>,
        filters: &serde_json::Map<String, Json>,
        keys: &[String],
    ) -> Result<Option<usize>> {
        let graph = iri(&self.graph)?;
        // Only positive filters that cannot hide normalized defaults narrow
        // discovery. P2 includes absent/invalid raw priority: do not narrow it.
        // Unassigned likewise needs all potential Action/status roots.
        // Ranges use broader roots too: never fan out into hundreds of queries.
        let assignees = req.filter.assignee.as_ref().map(|name| {
            vec![
                sparql_obj(&Obj::Str(name.clone())),
                sparql_obj(&Obj::Iri(vocab::principal_iri(name))),
            ]
        });
        let priorities = filters
            .get("priority")
            .and_then(Json::as_array)
            .filter(|values| values.len() == 1 && !values.contains(&Json::Null));
        let mut patterns = vec![String::new()];
        for (pred, values) in [
            (term::assigned_to(), assignees),
            (
                term::priority(),
                priorities.map(|vs| vs.iter().map(Json::to_string).collect()),
            ),
        ] {
            if let Some(values) = values {
                let mut next = Vec::new();
                for base in &patterns {
                    for v in &values {
                        next.push(format!("{base} ?s <{pred}> {v} ."));
                    }
                }
                patterns = next;
            }
        }
        let discovery = if patterns == [String::new()] {
            vec![
                format!("?s a <{}>", term::work_item()),
                format!("?s <{}> ?status", term::status()),
            ]
        } else {
            patterns
        };
        let mut roots = BTreeSet::new();
        // Separate indexed alternatives: a UNION of two otherwise fast joins
        // can force a whole-graph plan on older servers. No query LIMIT is a
        // completeness claim: each alternative and their union must be below it.
        for pattern in discovery {
            let query = format!(
                "SELECT DISTINCT ?s WHERE {{ GRAPH {graph} {{ {pattern} }} }} LIMIT {ROW_CAP}"
            );
            let Some(rows) = self.projection_rows(&query, &["s"])? else {
                return Ok(None);
            };
            let mut seen = BTreeSet::new();
            for row in rows {
                let Obj::Iri(s) = &row["s"] else {
                    return Err(invalid());
                };
                iri(s)?;
                if !seen.insert(s.clone()) {
                    return Err(invalid());
                }
                roots.insert(s.clone());
            }
            if roots.len() >= ROW_CAP {
                return Ok(None);
            }
        }
        let mut facts: Facts = roots.iter().map(|s| (s.clone(), BTreeMap::new())).collect();
        let mut predicates = BTreeSet::from([
            vocab::RDF_TYPE.to_string(),
            term::identifier(),
            term::status(),
        ]);
        predicates.extend(required.iter().map(|name| predicate(name)));
        let subjects: Vec<_> = roots.iter().collect();
        for pred in predicates {
            for chunk in subjects.chunks(SUBJECTS_PER_READ) {
                let branches = chunk
                    .iter()
                    .map(|s| {
                        Ok(format!(
                            "{{ BIND({0} AS ?s) GRAPH {graph} {{ {0} <{pred}> ?o }} }}",
                            iri(s)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join(" UNION ");
                let query = format!("SELECT ?s ?o WHERE {{ {branches} }} LIMIT {ROW_CAP}");
                let Some(rows) = self.projection_rows(&query, &["s", "o"])? else {
                    return Ok(None);
                };
                let allowed: BTreeSet<_> = chunk.iter().copied().collect();
                for row in rows {
                    let Obj::Iri(s) = &row["s"] else {
                        return Err(invalid());
                    };
                    if !allowed.contains(s) {
                        return Err(invalid());
                    }
                    facts
                        .get_mut(s)
                        .ok_or_else(invalid)?
                        .entry(pred.clone())
                        .or_default()
                        .push(row["o"].clone());
                }
            }
        }
        let mut ids = BTreeMap::new();
        let mut expected = 0usize;
        for (s, properties) in &facts {
            let values = |p: &str| properties.get(p).map(Vec::as_slice).unwrap_or(&[]);
            if !values(vocab::RDF_TYPE).contains(&Obj::Iri(term::work_item())) {
                return Ok(None);
            }
            let [Obj::Str(id)] = values(&term::identifier()) else {
                return Ok(None);
            };
            if ids.insert(id.clone(), s.clone()).is_some() {
                return Ok(None);
            }
            let [Obj::Str(status)] = values(&term::status()) else {
                return Ok(None);
            };
            for name in required {
                let vs = values(&predicate(name));
                if vs.len() > 1 {
                    return Ok(None);
                }
                if let Some(value) = vs.first() {
                    let valid = match *name {
                        "priority" => matches!(value, Obj::Int(n) if (0..=255).contains(n)),
                        "type" => matches!(value, Obj::Str(_)),
                        _ => {
                            matches!(value, Obj::Str(_))
                                || matches!(value, Obj::Iri(i) if vocab::principal_name(i).is_some())
                        }
                    };
                    if !valid {
                        return Ok(None);
                    }
                }
            }
            let assignee = values(&term::assigned_to()).first().and_then(|v| match v {
                Obj::Str(s) => Some(s.clone()),
                Obj::Iri(i) => vocab::principal_name(i),
                _ => None,
            });
            let priority = match values(&term::priority()).first() {
                Some(Obj::Int(n)) => *n,
                _ => 2,
            };
            let selected = keys.contains(status)
                && req
                    .filter
                    .assignee
                    .as_ref()
                    .is_none_or(|a| assignee.as_ref() == Some(a))
                && (!req.filter.unassigned || assignee.is_none())
                && filters
                    .get("priority")
                    .and_then(Json::as_array)
                    .is_none_or(|vs| vs.contains(&json!(priority)));
            if selected {
                expected += 1;
            }
        }
        // Positive owner lookups use exact ID terms, without a correlated join.
        // An extra non-Action owner conservatively falls back too; never hide a
        // duplicate Action outside the result's assignee/status/priority scope.
        let ids: Vec<_> = ids.into_iter().collect();
        for chunk in ids.chunks(SUBJECTS_PER_READ) {
            let branches = chunk
                .iter()
                .map(|(id, _)| {
                    let literal = sparql_obj(&Obj::Str(id.clone()));
                    format!(
                        "{{ BIND({literal} AS ?id) GRAPH {graph} {{ ?owner <{}> {literal} }} }}",
                        term::identifier()
                    )
                })
                .collect::<Vec<_>>()
                .join(" UNION ");
            let query = format!("SELECT ?id ?owner WHERE {{ {branches} }} LIMIT {ROW_CAP}");
            let Some(rows) = self.projection_rows(&query, &["id", "owner"])? else {
                return Ok(None);
            };
            let expected: BTreeMap<_, _> = chunk.iter().cloned().collect();
            let mut seen = BTreeSet::new();
            for row in rows {
                let (Obj::Str(id), Obj::Iri(owner)) = (&row["id"], &row["owner"]) else {
                    return Err(invalid());
                };
                let Some(subject) = expected.get(id) else {
                    return Err(invalid());
                };
                if owner != subject {
                    return Ok(None);
                }
                if !seen.insert(id.clone()) {
                    return Err(invalid());
                }
            }
            if seen.len() != expected.len() {
                return Err(invalid());
            }
        }
        Ok(Some(expected))
    }
}
