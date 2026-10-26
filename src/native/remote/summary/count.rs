//! A count and its groups must come from one server query snapshot.
use super::*;
#[cfg(test)]
mod cohort_tests;

impl RemoteBackend {
    pub(in crate::native::remote) fn sparql_count_summary(&self, req: &CountReq) -> Result<Count> {
        let (rows, truncated) = self.select_marked(&self.count_query(req)?, None)?;
        decode_count(rows, truncated, req.by.as_deref())
    }

    fn count_query(&self, req: &CountReq) -> Result<String> {
        let mut fields = self.filter_fields(&req.filter)?;
        if req.filter.status.is_none() {
            fields += " FILTER(?status != \"tombstone\")";
            if !req.include_closed {
                fields += " FILTER(?status != \"closed\")";
            }
        }
        let total = format!(
            "{{ {{ SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {} }} }} BIND(\"total\" AS ?arm) }}",
            self.effective_normalized(&fields)
        );
        let ambiguity = self.effective_normalized(&(fields.clone() + &ambiguity_guard(req)));
        let arms = if let Some(by) = req.by.as_deref() {
            let key = if by == "status" { "status" } else { "key" };
            let pattern = self.effective_normalized(&(fields + &Self::group_field(by)?));
            format!("{total} UNION {{ {{ SELECT ?{key} (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {pattern} }} GROUP BY ?{key} LIMIT {ROW_CAP} }} BIND(\"group\" AS ?arm) }}")
        } else {
            total
        };
        Ok(format!("SELECT ?arm ?status ?key ?n WHERE {{ {arms} UNION {{ {{ SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {ambiguity} }} }} BIND(\"ambiguous\" AS ?arm) }} }} LIMIT {ROW_CAP}"))
    }
}

// An aggregate cannot reproduce the model's first scalar from competing facts.
// Check the same effective IDs in the same query; a filtered-away competing
// assignee/status must not silently admit a different cohort. Labels remain
// genuinely multivalued and are deliberately absent from this guard.
fn ambiguity_guard(req: &CountReq) -> String {
    let mut conflicts = vec![format!("EXISTS {{ ?s <{}> ?count_other_id . FILTER(isLiteral(?count_other_id) && sameTerm(?count_other_id, STR(?count_other_id)) && ?count_other_id != ?id) }}", term::identifier())];
    conflicts.push(scalar_conflict(
        &term::status(),
        "status",
        "plain",
        Some("open"),
    ));
    let f = &req.filter;
    if f.assignee.is_some() || f.unassigned || req.by.as_deref() == Some("assignee") {
        conflicts.push(scalar_conflict(
            &term::assigned_to(),
            "assignee",
            "principal",
            None,
        ));
    }
    if f.priority.is_some()
        || f.priority_min.is_some()
        || f.priority_max.is_some()
        || req.by.as_deref() == Some("priority")
    {
        conflicts.push(scalar_conflict(
            &term::priority(),
            "priority",
            "priority",
            Some("2"),
        ));
    }
    if f.issue_type.is_some() || req.by.as_deref() == Some("type") {
        conflicts.push(scalar_conflict(
            &term::issue_type(),
            "type",
            "plain",
            Some("task"),
        ));
    }
    if f.parent.is_some() {
        conflicts.push(scalar_conflict(&term::child_of(), "parent", "iri", None));
    }
    for (active, predicate, name) in [
        (f.title_contains.is_some(), term::name(), "title"),
        (
            f.title_contains.is_some(),
            vocab::RDFS_LABEL.into(),
            "title_label",
        ),
        (
            f.desc_contains.is_some(),
            term::description(),
            "description",
        ),
        (f.notes_contains.is_some(), term::notes(), "notes"),
        (f.overdue_at.is_some(), term::due_at(), "due"),
    ] {
        if active {
            conflicts.push(scalar_conflict(&predicate, name, "plain", None));
        }
    }
    format!(" FILTER({})", conflicts.join(" || "))
}

fn scalar_conflict(predicate: &str, name: &str, kind: &str, default: Option<&str>) -> String {
    let mut patterns = String::new();
    for (source, side) in [("?s", "a"), ("?count_peer", "b")] {
        let raw = format!("?count_{name}_{side}_raw");
        let key = format!("?count_{name}_{side}");
        let guard = match kind {
            "principal" => principal_guard(&raw),
            "iri" => format!("isIRI({raw})"),
            "priority" => format!("sameTerm({raw} - {raw}, 0) && sameTerm({raw}, {raw} + 0) && {raw} >= -9223372036854775808 && {raw} <= 9223372036854775807"),
            _ => format!("isLiteral({raw}) && sameTerm({raw}, STR({raw}))"),
        };
        patterns += &format!(" OPTIONAL {{ {source} <{predicate}> {raw} . FILTER({guard}) }}");
        let value = match (kind, default) {
            ("priority", _) => {
                format!("IF(BOUND({raw}), IF({raw} >= 0 && {raw} <= 255, {raw}, 2), 2)")
            }
            (_, Some(default)) => format!("COALESCE({raw}, {})", literal(default)),
            _ => raw,
        };
        patterns += &format!(" BIND({value} AS {key})");
    }
    let a = format!("?count_{name}_a");
    let b = format!("?count_{name}_b");
    format!("EXISTS {{ ?count_peer a <{}> ; <{}> ?id . {patterns} FILTER((BOUND({a}) && !BOUND({b})) || (!BOUND({a}) && BOUND({b})) || (BOUND({a}) && BOUND({b}) && !sameTerm({a}, {b}))) }}", term::work_item(), term::identifier())
}

fn number(row: &BTreeMap<String, Obj>) -> Result<usize> {
    match row.get("n") {
        Some(Obj::Int(n)) => usize::try_from(*n).ok(),
        Some(Obj::Str(n)) => n.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| SdError::failed("quipu returned an invalid count"))
}

fn decode_count(
    rows: Vec<BTreeMap<String, Obj>>,
    truncated: bool,
    by: Option<&str>,
) -> Result<Count> {
    if truncated || rows.len() >= ROW_CAP {
        return Err(SdError::failed(
            "quipu truncated counts; refusing an incomplete summary",
        ));
    }
    let mut total = None;
    let mut ambiguity = None;
    let mut groups = BTreeMap::new();
    for row in rows {
        let n = number(&row)?;
        match row.get("arm") {
            Some(Obj::Str(arm)) if arm == "ambiguous" => {
                if ambiguity.replace(n).is_some() || n != 0 {
                    return Err(SdError::failed(
                        "quipu count cohort has ambiguous scalar facts",
                    ));
                }
            }
            Some(Obj::Str(arm)) if arm == "total" => {
                if total.replace(n).is_some() {
                    return Err(SdError::failed("quipu returned duplicate count totals"));
                }
            }
            Some(Obj::Str(arm)) if arm == "group" && by.is_some() => {
                if n == 0 {
                    return Err(SdError::failed("quipu returned an empty count group"));
                }
                let key = if by == Some("status") {
                    "status"
                } else {
                    "key"
                };
                let value = match row.get(key) {
                    Some(Obj::Str(s)) => s.clone(),
                    Some(Obj::Iri(s)) if by == Some("assignee") => vocab::principal_name(s)
                        .ok_or_else(|| SdError::failed("quipu returned a foreign principal"))?,
                    _ => return Err(SdError::failed("quipu returned an invalid group key")),
                };
                // Different RDF representations can name the same principal.
                // Adding their aggregates would double-count overlapping IDs.
                if groups.insert(value, n).is_some() {
                    return Err(SdError::failed("quipu returned ambiguous count groups"));
                }
            }
            _ => return Err(SdError::failed("quipu returned an invalid count arm")),
        }
    }
    let total = total.ok_or_else(|| SdError::failed("quipu omitted the count total"))?;
    if ambiguity != Some(0) {
        return Err(SdError::failed("quipu omitted the count ambiguity check"));
    }
    if groups.values().any(|n| *n > total) {
        return Err(SdError::failed(
            "quipu returned a group larger than its count total",
        ));
    }
    if by.is_some() && total > 0 && groups.is_empty() {
        return Err(SdError::failed("quipu omitted nonempty count groups"));
    }
    if by.is_some() && by != Some("label") {
        let sum = groups
            .values()
            .try_fold(0usize, |sum, n| sum.checked_add(*n));
        if sum != Some(total) {
            return Err(SdError::failed(
                "quipu returned incomplete or ambiguous count groups",
            ));
        }
    }
    Ok(Count {
        total,
        groups: by.map(|_| groups.into_iter().collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(arm: &str, key: Option<Obj>, n: i64) -> BTreeMap<String, Obj> {
        let mut row = BTreeMap::from([
            ("arm".into(), Obj::Str(arm.into())),
            ("n".into(), Obj::Int(n)),
        ]);
        if let Some(key) = key {
            row.insert("key".into(), key);
        }
        row
    }

    #[test]
    fn count_snapshot_empty_and_overlapping_labels_are_valid() {
        let empty = decode_count(
            vec![row("total", None, 0), row("ambiguous", None, 0)],
            false,
            Some("assignee"),
        )
        .unwrap();
        assert_eq!(empty.total, 0);
        assert_eq!(empty.groups, Some(vec![]));
        let labels = decode_count(
            vec![
                row("total", None, 2),
                row("ambiguous", None, 0),
                row("group", Some(Obj::Str("a".into())), 2),
                row("group", Some(Obj::Str("b".into())), 2),
            ],
            false,
            Some("label"),
        )
        .unwrap();
        assert_eq!(labels.total, 2);
        assert_eq!(labels.groups.unwrap().len(), 2);
    }

    #[test]
    fn count_snapshot_unknown_or_partial_is_never_zero() {
        let total = row("total", None, 2);
        let group = row("group", Some(Obj::Str("P1".into())), 1);
        for rows in [
            vec![],
            vec![total.clone(), total.clone()],
            vec![group.clone()],
            vec![total.clone(), group.clone()],
            vec![row("unknown", None, 0)],
            vec![row("total", None, -1)],
            vec![total.clone(), row("group", None, 2)],
        ] {
            assert!(decode_count(rows, false, Some("priority")).is_err());
        }
        assert!(decode_count(vec![total.clone()], true, None).is_err());
        assert!(decode_count(
            vec![total.clone(), row("ambiguous", None, 0)],
            false,
            Some("label")
        )
        .is_err());
        assert!(decode_count(vec![total, group], false, None).is_err());
    }

    #[test]
    fn count_snapshot_principal_aliases_cannot_be_added_twice() {
        let rows = vec![
            row("total", None, 2),
            row("ambiguous", None, 0),
            row("group", Some(Obj::Str("reviewer one".into())), 1),
            row(
                "group",
                Some(Obj::Iri(vocab::principal_iri("reviewer one"))),
                1,
            ),
        ];
        assert!(decode_count(rows, false, Some("assignee")).is_err());
    }
}
