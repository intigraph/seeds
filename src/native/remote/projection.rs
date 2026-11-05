//! Optional current-board aggregate reads. Raw projection rows are not the
//! normalized Seeds model: prove compatibility on each call, never cache it.
use super::*;
use crate::engine::{Count, CountReq, Filter};
mod scoped;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CountSource {
    Projection,
    Sparql,
}

fn invalid() -> SdError {
    SdError::failed("quipu returned an invalid board projection response")
}

fn number(v: &Json) -> Result<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(invalid)
}

fn aggregate(v: &Json) -> Result<(u64, usize)> {
    if v["truncated"].as_bool() != Some(false) {
        return Err(invalid());
    }
    Ok((v["tx"].as_u64().ok_or_else(invalid)?, number(&v["total"])?))
}

impl RemoteBackend {
    // A missing route is an older server, not an empty board. Auth, timeout,
    // transport and server failures remain errors, never capability absence.
    fn projection_read(&self, body: Json) -> Result<Option<Json>> {
        let url = format!("{}/projections", self.base);
        match quipu_read_request(self.agent.post(&url), "projections")
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_string(&body.to_string())
        {
            Ok(r) => {
                let text = r.into_string().map_err(|_| invalid())?;
                serde_json::from_str(&text).map(Some).map_err(|_| invalid())
            }
            Err(ureq::Error::Status(404 | 405, _)) => Ok(None),
            Err(ureq::Error::Status(code, r)) => Err(status_error(
                "projections",
                code,
                &r.into_string().unwrap_or_default(),
            )),
            Err(ureq::Error::Transport(t)) => Err(transport(&url, &t)),
        }
    }

    #[cfg(all(test, feature = "projection-contract-tests"))]
    fn original_projection_guard(&self, required: &BTreeSet<&str>) -> Result<bool> {
        let st = term::status();
        let id = term::identifier();
        let action = term::work_item();
        let plain = |v: &str| format!("isLiteral({v}) && sameTerm({v}, STR({v}))");
        let mut anomalies = vec![
            format!("{{ ?s <{st}> ?v FILTER(!({})) }}",plain("?v")),
            format!("{{ ?s <{st}> ?v FILTER NOT EXISTS {{ ?s a <{action}> ; <{id}> ?id FILTER({}) }} }}",plain("?id")),
            format!("{{ ?s a <{action}> ; <{id}> ?id FILTER({}) FILTER NOT EXISTS {{ ?s <{st}> ?v }} }}",plain("?id")),
            format!("{{ ?s a <{action}> ; <{id}> ?id . ?other a <{action}> ; <{id}> ?id FILTER(?s != ?other && {}) }}",plain("?id")),
            format!("{{ ?s <{st}> ?v ; <{id}> ?a, ?b FILTER(!sameTerm(?a, ?b)) }}"),
        ];
        for name in required {
            let (predicate, valid) = match *name {
                "priority" => (
                    term::priority(),
                    format!("{} && ?v <= 255", summary::nonnegative_integer_guard("?v")),
                ),
                "type" => (term::issue_type(), plain("?v")),
                _ => (
                    term::assigned_to(),
                    format!(
                        "{} || (isIRI(?v) && STRSTARTS(STR(?v), \"{}principal/\"))",
                        plain("?v"),
                        vocab::SEEDS_BASE
                    ),
                ),
            };
            anomalies.push(format!(
                "{{ ?s <{st}> ?status ; <{predicate}> ?bad FILTER NOT EXISTS {{ ?s <{predicate}> ?v FILTER(sameTerm(?bad, ?v) && ({valid})) }} }}"
            ));
        }
        let body = self.post("/query", "application/json", &json!({"query":format!("ASK {{ GRAPH <{}> {{ {} }} }}",self.graph,anomalies.join(" UNION "))}).to_string(), false)?;
        if ask_answer(&self.graph, &body)? {
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn projection_count(&self, req: &CountReq) -> Result<Option<Count>> {
        // Only declared scalar column filters are representable. Joins and
        // all other scopes retain the normalized SPARQL path.
        let mut filter = req.filter.clone();
        filter.status = None;
        filter.assignee = None;
        filter.unassigned = false;
        filter.priority = None;
        filter.priority_min = None;
        filter.priority_max = None;
        if filter != Filter::default() || req.by.as_deref() == Some("label") {
            return Ok(None);
        }
        let by = req.by.as_deref().unwrap_or("status");
        if !matches!(by, "status" | "priority" | "type" | "assignee") {
            return Ok(None);
        }
        let Some(list) = self.projection_read(json!({"action":"list"}))? else {
            return Ok(None);
        };
        let declarations = list["declared"].as_array().ok_or_else(invalid)?;
        let boards = declarations
            .iter()
            .filter(|d| d["name"] == "board")
            .collect::<Vec<_>>();
        if boards.is_empty() {
            return Ok(None);
        }
        if boards.len() != 1 || boards[0]["key"] != term::status() {
            return Ok(None);
        }
        let def = boards[0];
        let mut predicates = serde_json::Map::new();
        if req.filter.assignee.is_some() && req.filter.unassigned {
            return Ok(None);
        }
        if let Some(person) = &req.filter.assignee {
            predicates.insert(
                "assignee".into(),
                json!([person, {"iri":vocab::principal_iri(person)}]),
            );
        } else if req.filter.unassigned {
            predicates.insert("assignee".into(), json!([null]));
        }
        if req.filter.priority.is_some()
            || req.filter.priority_min.is_some()
            || req.filter.priority_max.is_some()
        {
            let parse =
                |v: &Option<String>| v.as_deref().map(crate::model::parse_priority).transpose();
            let exact = parse(&req.filter.priority)?;
            let min = parse(&req.filter.priority_min)?.unwrap_or(0);
            let max = parse(&req.filter.priority_max)?.unwrap_or(u8::MAX);
            let mut values: Vec<Json> = (min..=max)
                .filter(|p| exact.is_none_or(|e| *p == e))
                .map(|p| json!(p))
                .collect();
            if values.is_empty() {
                return Ok(None);
            }
            // Seeds defaults a missing priority to P2.
            if values.contains(&json!(2)) {
                values.push(Json::Null);
            }
            predicates.insert("priority".into(), json!(values));
        }
        let mut required = BTreeSet::new();
        if by != "status" {
            required.insert(by);
        }
        required.extend(predicates.keys().map(String::as_str));
        for name in &required {
            let predicate = match *name {
                "priority" => term::priority(),
                "type" => term::issue_type(),
                _ => term::assigned_to(),
            };
            let columns = def["columns"].as_array().ok_or_else(invalid)?;
            if columns.iter().filter(|c| c["name"] == *name).count() != 1
                || !columns
                    .iter()
                    .any(|c| c["name"] == *name && c["predicate"] == predicate)
            {
                return Ok(None);
            }
        }
        let request = |group: &str, keys: &[String]| json!({"name":"board","graph":self.graph,"group_by":group,"key_in":keys,"max_groups":ROW_CAP});
        let Some(before) = self.projection_read(request("key", &[]))? else {
            return Ok(None);
        };
        let (tx, total) = aggregate(&before)?;
        if number(&before["multi_valued_rows"])? != 0 {
            return Ok(None);
        }
        let statuses = before["groups"].as_array().ok_or_else(invalid)?;
        let mut keys = Vec::new();
        let mut sum = 0usize;
        let mut seen = BTreeSet::new();
        let status = req
            .filter
            .status
            .as_deref()
            .map(crate::model::parse_status)
            .transpose()?;
        for row in statuses {
            let value = row["value"].as_str().ok_or_else(invalid)?;
            if !seen.insert(value) {
                return Err(invalid());
            }
            sum = sum.checked_add(number(&row["n"])?).ok_or_else(invalid)?;
            if status.as_deref().map_or(
                value != "tombstone" && (req.include_closed || value != "closed"),
                |s| value == s,
            ) {
                keys.push(value.to_string());
            }
        }
        if sum != total {
            return Err(invalid());
        }
        if !predicates.is_empty() {
            // Older projection servers silently ignore unknown body fields.
            // An empty IN must reject every row of a nonempty board. Prove
            // this on every call, bracketed by the same transaction watermark.
            if total == 0 {
                return Ok(None);
            }
            let mut probe = request("key", &[]);
            probe["where"] = json!({"key":[]});
            let Some(proof) = self.projection_read(probe)? else {
                return Ok(None);
            };
            let (proof_tx, proof_total) = aggregate(&proof)?;
            if proof_tx != tx || proof_total != 0 {
                return Ok(None);
            }
            if !proof["groups"].as_array().ok_or_else(invalid)?.is_empty() {
                return Err(invalid());
            }
        }
        if self.ephemeral_may_hold_facts()? {
            return Ok(None);
        }
        let Some(expected_total) = self.projection_scope(req, &required, &predicates, &keys)?
        else {
            return Ok(None);
        };
        // Compare watermarks around ALL compatibility reads. A concurrent
        // commit (including an ephemeral write) discards the optimization.
        let group = if by == "status" || keys.is_empty() {
            "key"
        } else {
            by
        };
        let mut body = request(group, &keys);
        if !predicates.is_empty() {
            body["where"] = json!(predicates);
        }
        let Some(after) = self.projection_read(body)? else {
            return Ok(None);
        };
        let (after_tx, after_total) = aggregate(&after)?;
        if after_tx != tx || number(&after["multi_valued_rows"])? != 0 {
            return Ok(None);
        }
        let mut groups = BTreeMap::<String, usize>::new();
        let mut projected_total = 0usize;
        let mut raw_keys = BTreeSet::new();
        for row in after["groups"].as_array().ok_or_else(invalid)? {
            if !raw_keys.insert(row["value"].to_string()) {
                return Err(invalid());
            }
            let n = number(&row["n"])?;
            projected_total = projected_total.checked_add(n).ok_or_else(invalid)?;
            if keys.is_empty() {
                continue;
            }
            let value = &row["value"];
            let key = match by {
                "status" => value.as_str().ok_or_else(invalid)?.to_string(),
                "priority" => format!(
                    "P{}",
                    if value.is_null() {
                        2
                    } else {
                        value.as_u64().filter(|n| *n <= 255).ok_or_else(invalid)?
                    }
                ),
                "type" => {
                    if value.is_null() {
                        "task".into()
                    } else {
                        value.as_str().ok_or_else(invalid)?.into()
                    }
                }
                _ => {
                    if value.is_null() {
                        "(unassigned)".into()
                    } else if let Some(iri) = value["iri"].as_str() {
                        vocab::principal_name(iri).ok_or_else(invalid)?
                    } else {
                        value.as_str().ok_or_else(invalid)?.into()
                    }
                }
            };
            *groups.entry(key).or_default() += n;
        }
        if projected_total != after_total || (!keys.is_empty() && after_total != expected_total) {
            return Err(invalid());
        }
        Ok(Some(Count {
            total: if keys.is_empty() { 0 } else { after_total },
            groups: req.by.as_ref().map(|_| groups.into_iter().collect()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Filter;

    #[cfg(feature = "projection-contract-tests")]
    const CONFIG: &str = r#"
[quipu.shacl]
validate_on_write=false
[quipu.embedding]
auto_embed=false
[[quipu.projections]]
name="board"
key="https://seeds.local/ontology/status"
[[quipu.projections.columns]]
name="priority"
predicate="https://seeds.local/ontology/priority"
[[quipu.projections.columns]]
name="type"
predicate="https://seeds.local/ontology/issueType"
[[quipu.projections.columns]]
name="assignee"
predicate="https://schema.org/agent"
"#;

    fn update(remote: &RemoteBackend, graph: &str, triples: &str) {
        let query = format!("INSERT DATA {{ GRAPH <{graph}> {{ {triples} }} }}");
        remote
            .post(
                "/update",
                "application/x-www-form-urlencoded",
                &format!("update={}", form_encode(&query)),
                true,
            )
            .unwrap();
    }

    fn seed(id: &str, status: &str, priority: &str) -> String {
        format!("<https://seeds.local/item/{id}> a <{}> ; <{}> \"{id}\" ; <{}> \"{status}\" {priority} .",term::work_item(),term::identifier(),term::status())
    }

    fn parity(remote: &RemoteBackend, req: &CountReq) -> CountSource {
        let (actual, source) = remote.count_summary_with_source(req).unwrap();
        let reference = remote.sparql_count_summary(req).unwrap();
        assert_eq!(actual.total, reference.total, "{req:?}");
        assert_eq!(actual.groups, reference.groups, "{req:?}");
        source
    }

    #[cfg(feature = "projection-contract-tests")]
    #[test]
    fn projection_count_real_parity_and_semantic_fallbacks() {
        let bin = std::env::var("SEEDS_TEST_PROJECTIONS_SERVER")
            .expect("projection contract requires SEEDS_TEST_PROJECTIONS_SERVER");
        let server = super::super::tests::quipu_server_config(&bin, Some(CONFIG));
        let graph = "https://seeds.local/project/projection-test";
        let remote = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        let p = format!(
            "; <{}> 1 ; <{}> <{}>",
            term::priority(),
            term::assigned_to(),
            vocab::principal_iri("reader")
        );
        let data = seed("a", "open", &p)
            + &seed("b", "closed", &p)
            + &seed("c", "tombstone", &p)
            + &seed("d", "deferred", &p)
            + &seed("e", "open", "");
        update(&remote, graph, &data);
        let mut cases = 0;
        for by in [
            None,
            Some("status"),
            Some("priority"),
            Some("type"),
            Some("assignee"),
        ] {
            for status in [
                None,
                Some("open"),
                Some("closed"),
                Some("tombstone"),
                Some("deferred"),
                Some("blocked"),
            ] {
                for include_closed in [false, true] {
                    let req = CountReq {
                        by: by.map(str::to_string),
                        include_closed,
                        filter: Filter {
                            status: status.map(str::to_string),
                            ..Filter::default()
                        },
                    };
                    assert_eq!(parity(&remote, &req), CountSource::Projection, "{req:?}");
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 60);
        // Deferral dates do not turn deferred work into open work for count.
        update(
            &remote,
            graph,
            &format!(
                "<https://seeds.local/item/d> <{}> \"2000-01-01T00:00:00Z\" .",
                term::defer_until()
            ),
        );
        assert_eq!(
            parity(&remote, &CountReq::default()),
            CountSource::Projection
        );
        let scoped = CountReq {
            filter: Filter {
                assignee: Some("reader".into()),
                priority: Some("1".into()),
                ..Filter::default()
            },
            by: Some("status".into()),
            ..CountReq::default()
        };
        let scoped_source = parity(&remote, &scoped);
        assert_eq!(remote.count_summary(&scoped).unwrap().total, 2);
        // A plain status missing on a valid item must still default to open.
        update(
            &remote,
            graph,
            &format!(
                "<https://seeds.local/item/missing> a <{}> ; <{}> \"missing\" .",
                term::work_item(),
                term::identifier()
            ),
        );
        assert_eq!(parity(&remote, &CountReq::default()), CountSource::Sparql);
        // Isolated anomalous rows must not borrow the preceding missing-status fallback.
        for (suffix, extra, by) in [
            (
                "duplicate",
                format!(
                    "<https://seeds.local/item/other> a <{}> ; <{}> \"bad\" ; <{}> \"open\" .",
                    term::work_item(),
                    term::identifier(),
                    term::status()
                ),
                "status",
            ),
            (
                "multistatus",
                format!(
                    "<https://seeds.local/item/bad> <{}> \"closed\" .",
                    term::status()
                ),
                "status",
            ),
            (
                "typedpriority",
                format!(
                    "<https://seeds.local/item/bad> <{}> \"high\" .",
                    term::priority()
                ),
                "priority",
            ),
        ] {
            let g = format!("{graph}-{suffix}");
            let b = RemoteBackend::connect(&server.base, &g, None, None, &[]).unwrap();
            update(&b, &g, &(seed("bad", "open", "") + &extra));
            let req = CountReq {
                by: Some(by.into()),
                ..CountReq::default()
            };
            assert_eq!(parity(&b, &req), CountSource::Sparql, "{suffix}");
        }
        // Ephemeral replaces the main identity, never adds a second count.
        update(
            &remote,
            &vocab::ephemeral_graph(graph),
            &seed("a", "closed", &p),
        );
        assert_eq!(parity(&remote, &CountReq::default()), CountSource::Sparql);
        let fallbacks = 5 + usize::from(scoped_source == CountSource::Sparql);
        eprintln!(
            "COUNT_READER source=projection parity_cases={cases} semantic_fallbacks={fallbacks}"
        );
    }

    #[cfg(feature = "projection-contract-tests")]
    #[test]
    fn projection_count_where_real_parity() {
        let bin = std::env::var("SEEDS_TEST_PROJECTIONS_SERVER")
            .expect("where contract requires SEEDS_TEST_PROJECTIONS_SERVER");
        let server = super::super::tests::quipu_server_config(&bin, Some(CONFIG));
        let graph = "https://seeds.local/project/where-test";
        let remote = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        let mut data = String::new();
        let mut index = 0;
        for status in ["open", "in_progress", "closed", "tombstone", "deferred"] {
            for priority in [Some(1), Some(2), None] {
                for assignee in [
                    Some(format!("<{}>", vocab::principal_iri("reader"))),
                    Some("\"reader\"".into()),
                    Some("\"other\"".into()),
                    None,
                ] {
                    let mut fields = String::new();
                    if let Some(p) = priority {
                        fields += &format!("; <{}> {p}", term::priority());
                    }
                    if let Some(a) = assignee {
                        fields += &format!("; <{}> {a}", term::assigned_to());
                    }
                    data += &seed(&format!("where-{index}"), status, &fields);
                    index += 1;
                }
            }
        }
        update(&remote, graph, &data);
        let mut cases = 0;
        for assignee in [None, Some("reader"), Some("unknown"), Some("(unassigned)")] {
            for priority in [None, Some("1"), Some("P2"), Some("range")] {
                for status in [None, Some("open"), Some("closed"), Some("tombstone")] {
                    for include_closed in [false, true] {
                        let req = CountReq {
                            by: Some("status".into()),
                            include_closed,
                            filter: Filter {
                                assignee: assignee
                                    .filter(|a| *a != "(unassigned)")
                                    .map(str::to_string),
                                unassigned: assignee == Some("(unassigned)"),
                                priority: priority.filter(|p| *p != "range").map(str::to_string),
                                priority_min: (priority == Some("range")).then(|| "1".into()),
                                priority_max: (priority == Some("range")).then(|| "2".into()),
                                status: status.map(str::to_string),
                                ..Filter::default()
                            },
                        };
                        assert_eq!(parity(&remote, &req), CountSource::Projection, "{req:?}");
                        cases += 1;
                    }
                }
            }
        }
        assert_eq!(cases, 128);
        let broad_range = CountReq {
            by: Some("status".into()),
            filter: Filter {
                assignee: Some("reader".into()),
                priority_min: Some("3".into()),
                ..Filter::default()
            },
            ..CountReq::default()
        };
        assert_eq!(parity(&remote, &broad_range), CountSource::Projection);

        let scoped = CountReq {
            by: Some("status".into()),
            filter: Filter {
                assignee: Some("reader".into()),
                priority: Some("1".into()),
                ..Filter::default()
            },
            ..CountReq::default()
        };
        assert_eq!(remote.count_summary(&scoped).unwrap().total, 6);
        // Validation covers filtered columns even when grouping by status.
        for (name, extra) in [
            ("bad-priority", format!("; <{}> \"high\"", term::priority())),
            (
                "bad-assignee",
                format!(
                    "; <{}> <https://example.test/not-a-principal>",
                    term::assigned_to()
                ),
            ),
            ("multi-priority", format!("; <{}> 1, 2", term::priority())),
            (
                "multi-assignee",
                format!("; <{}> \"reader\", \"other\"", term::assigned_to()),
            ),
        ] {
            let g = format!("{graph}-{name}");
            let b = RemoteBackend::connect(&server.base, &g, None, None, &[]).unwrap();
            let selected = format!(
                "; <{}> 1 ; <{}> \"reader\" {extra}",
                term::priority(),
                term::assigned_to()
            );
            update(&b, &g, &seed("bad", "open", &selected));
            assert_eq!(parity(&b, &scoped), CountSource::Sparql, "{name}");
        }
        update(
            &remote,
            &vocab::ephemeral_graph(graph),
            &seed("where-0", "closed", ""),
        );
        assert_eq!(parity(&remote, &scoped), CountSource::Sparql);
        eprintln!(
            "COUNT_WHERE source=projection parity_cases={cases} broad_range=1 semantic_fallbacks=5"
        );
    }

    #[cfg(feature = "projection-contract-tests")]
    #[test]
    fn projection_count_scoped_guard_negatives() {
        let bin = std::env::var("SEEDS_TEST_PROJECTIONS_SERVER")
            .expect("real projection server required");
        let server = super::super::tests::quipu_server_config(&bin, Some(CONFIG));
        let req = CountReq {
            by: Some("status".into()),
            filter: Filter {
                assignee: Some("reader".into()),
                priority: Some("1".into()),
                ..Filter::default()
            },
            ..CountReq::default()
        };
        let p = format!(
            "; <{}> 1 ; <{}> <{}>",
            term::priority(),
            term::assigned_to(),
            vocab::principal_iri("reader")
        );
        let subject = "<https://seeds.local/item/bad>";
        let cases = vec![
            ("missing-action", format!("DELETE DATA {{ GRAPH <G> {{ {subject} a <{}> }} }}",term::work_item())),
            ("missing-id", format!("DELETE DATA {{ GRAPH <G> {{ {subject} <{}> \"bad\" }} }}",term::identifier())),
            ("missing-status", format!("DELETE DATA {{ GRAPH <G> {{ {subject} <{}> \"open\" }} }}",term::status())),
            ("status-language", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> \"open\"@en }} }}",term::status())),
            ("multiple-id", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> \"other\" }} }}",term::identifier())),
            ("duplicate-outside-filter", format!("INSERT DATA {{ GRAPH <G> {{ <urn:other-owner> a <{}> ; <{}> \"bad\" ; <{}> 2 }} }}",term::work_item(),term::identifier(),term::priority())),
            ("priority-arithmetic-error", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> \"high\" }} }}",term::priority())),
            ("priority-decimal", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> \"1.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> }} }}",term::priority())),
            ("assignee-foreign-iri", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> <urn:foreign-person> }} }}",term::assigned_to())),
            ("id-language", format!("INSERT DATA {{ GRAPH <G> {{ {subject} <{}> \"bad\"@en }} }}",term::identifier())),
        ];
        for (name, mutation) in cases {
            let graph = format!("https://seeds.local/project/scoped-{name}");
            let remote = RemoteBackend::connect(&server.base, &graph, None, None, &[]).unwrap();
            update(&remote, &graph, &seed("bad", "open", &p));
            assert!(
                remote
                    .original_projection_guard(&BTreeSet::from(["assignee", "priority"]))
                    .unwrap(),
                "{name} before"
            );
            let mutation = mutation.replace("<G>", &format!("<{graph}>"));
            remote
                .post(
                    "/update",
                    "application/x-www-form-urlencoded",
                    &format!("update={}", form_encode(&mutation)),
                    true,
                )
                .unwrap();
            assert!(
                !remote
                    .original_projection_guard(&BTreeSet::from(["assignee", "priority"]))
                    .unwrap(),
                "{name} original"
            );
            assert!(
                remote.projection_count(&req).unwrap().is_none(),
                "{name} scoped"
            );
            assert_eq!(
                parity(&remote, &req),
                CountSource::Sparql,
                "{name} canonical"
            );
        }
        // An anomaly outside the selected roots must not poison an otherwise
        // normalized count. Its duplicate ID owners would still be checked.
        let graph = "https://seeds.local/project/scoped-outside";
        let remote = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        update(
            &remote,
            graph,
            &(seed("bad", "open", &p)
                + &format!(
                    "<urn:unrelated> a <{}> ; <{}> \"outside\" .",
                    term::work_item(),
                    term::identifier()
                )),
        );
        assert!(!remote
            .original_projection_guard(&BTreeSet::from(["assignee", "priority"]))
            .unwrap());
        assert_eq!(parity(&remote, &req), CountSource::Projection);
        // Missing priority P2 and unassigned cannot use positive-only roots.
        let defaults = CountReq {
            filter: Filter {
                unassigned: true,
                priority: Some("2".into()),
                ..Filter::default()
            },
            ..CountReq::default()
        };
        assert_eq!(parity(&remote, &defaults), CountSource::Sparql);
        eprintln!("SCOPED_GUARDS original_negative_cases=10 outside_scope=1 default_guard=1");
    }

    #[test]
    fn projection_count_scoped_rejects_incomplete_or_malformed_reads() {
        let req = CountReq::default();
        for (code, body, error) in [
            (403, "refused", true),
            (408, "timeout", true),
            (200, r#"{"boolean":false}"#, true),
            (
                200,
                r#"{"results":{"bindings":[{"s":{"type":"literal","value":"not-iri"}}]}}"#,
                true,
            ),
            (200, r#"{"results":{"bindings":[{}]}}"#, true),
            (
                200,
                r#"{"truncated":true,"results":{"bindings":[]}}"#,
                false,
            ),
        ] {
            let (base, worker) =
                super::super::gateway_tests::server(vec![(200, "ok"), (code, body)]);
            let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
            let result =
                remote.projection_scope(&req, &BTreeSet::new(), &serde_json::Map::new(), &[]);
            if error {
                assert!(result.is_err(), "{code}/{body}");
            } else {
                assert!(result.unwrap().is_none());
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn projection_count_scoped_full_limit_is_not_an_empty_answer() {
        let rows = vec![json!({"s":{"type":"uri","value":"urn:seed"}}); ROW_CAP];
        let body = json!({"results":{"bindings":rows}}).to_string();
        let (base, worker) = super::super::gateway_tests::server(vec![
            (200, "ok"),
            (200, Box::leak(body.into_boxed_str())),
        ]);
        let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
        assert!(remote
            .projection_scope(
                &CountReq::default(),
                &BTreeSet::new(),
                &serde_json::Map::new(),
                &[]
            )
            .unwrap()
            .is_none());
        worker.join().unwrap();
    }

    fn scope_responses(filtered: bool) -> Vec<(u16, String)> {
        let uri = |value: &str| json!({"type":"uri","value":value});
        let lit = |value: &str| json!({"type":"literal","value":value});
        let s = uri("urn:seed");
        let mut rows = vec![
            json!({"s":s}),
            json!({"s":s}),
            json!({"s":s,"o":uri(&term::work_item())}),
        ];
        if filtered {
            rows.push(json!({"s":s,"o":uri(&vocab::principal_iri("reader"))}));
        }
        rows.push(json!({"s":s,"o":lit("a")}));
        if filtered {
            rows.push(json!({"s":s,"o":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":"1"}}));
        }
        rows.push(json!({"s":s,"o":lit("open")}));
        rows.push(json!({"id":lit("a"),"owner":s}));
        rows.into_iter()
            .map(|row| (200, json!({"results":{"bindings":[row]}}).to_string()))
            .collect()
    }

    #[test]
    fn projection_count_where_detects_ignored_fields_and_races() {
        const LIST: &str = r#"{"declared":[{"name":"board","key":"https://seeds.local/ontology/status","columns":[{"name":"priority","predicate":"https://seeds.local/ontology/priority"},{"name":"assignee","predicate":"https://schema.org/agent"}]}]}"#;
        const BEFORE: &str = r#"{"tx":1,"total":1,"truncated":false,"multi_valued_rows":0,"groups":[{"value":"open","n":1}]}"#;
        const EMPTY: &str = r#"{"tx":1,"total":0,"truncated":false,"groups":[]}"#;
        let req = CountReq {
            by: Some("status".into()),
            filter: Filter {
                assignee: Some("reader".into()),
                priority: Some("1".into()),
                ..Filter::default()
            },
            ..CountReq::default()
        };
        for (code, proof, error) in [
            (200, BEFORE, false), // Old projection route ignores where.
            (
                200,
                r#"{"tx":2,"total":0,"truncated":false,"groups":[]}"#,
                false,
            ),
            (408, "budget exceeded", true),
            (
                200,
                r#"{"tx":1,"total":0,"truncated":false,"groups":[{}]}"#,
                true,
            ),
        ] {
            let (base, worker) = super::super::gateway_tests::server(vec![
                (200, "ok"),
                (200, LIST),
                (200, BEFORE),
                (code, proof),
            ]);
            let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
            let result = remote.projection_count(&req);
            if error {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_none());
            }
            let requests = worker.join().unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[3].contains("\"where\":{\"key\":[]}"));
        }
        let mut replies = vec![
            (200, "ok".to_string()),
            (200, LIST.into()),
            (200, BEFORE.into()),
            (200, EMPTY.into()),
            (200, r#"{"boolean":false}"#.into()),
        ];
        replies.extend(scope_responses(true));
        replies.push((200, BEFORE.into()));
        let refs = replies
            .into_iter()
            .map(|(code, body)| (code, &*Box::leak(body.into_boxed_str())))
            .collect();
        let (base, worker) = super::super::gateway_tests::server(refs);
        let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
        assert_eq!(remote.projection_count(&req).unwrap().unwrap().total, 1);
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 14);
        let body: Json = serde_json::from_str(requests[13].split_once("\r\n").unwrap().1).unwrap();
        assert_eq!(body["where"]["priority"], json!([1]));
        assert_eq!(
            body["where"]["assignee"],
            json!(["reader", {"iri":vocab::principal_iri("reader")} ])
        );
    }

    #[test]
    fn projection_count_legacy_server_reports_sparql_fallback() {
        let Some(server) = super::super::tests::quipu_server() else {
            return;
        };
        let graph = "https://seeds.local/project/no-projection";
        let remote = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        update(
            &remote,
            graph,
            &(seed("a", "open", "")
                + &seed("b", "closed", "")
                + &seed("c", "tombstone", "")
                + &seed("d", "deferred", "")
                + &seed("e", "open", "")),
        );
        let mut cases = 0;
        for by in [
            None,
            Some("status"),
            Some("priority"),
            Some("type"),
            Some("assignee"),
        ] {
            for status in [
                None,
                Some("open"),
                Some("closed"),
                Some("tombstone"),
                Some("deferred"),
                Some("blocked"),
            ] {
                for include_closed in [false, true] {
                    let req = CountReq {
                        by: by.map(str::to_string),
                        include_closed,
                        filter: Filter {
                            status: status.map(str::to_string),
                            ..Filter::default()
                        },
                    };
                    assert_eq!(parity(&remote, &req), CountSource::Sparql, "{req:?}");
                    cases += 1;
                }
            }
        }
        assert_eq!(cases, 60);
        assert_eq!(remote.count_summary(&CountReq::default()).unwrap().total, 3);
        eprintln!("COUNT_READER source=sparql fallback=true parity_cases={cases}");
    }
    #[test]
    fn projection_count_route_absence_and_errors_are_distinct() {
        for code in [404, 405, 401, 403, 408, 500, 503] {
            let (base, worker) =
                super::super::gateway_tests::server(vec![(200, "ok"), (code, "unavailable")]);
            let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
            let result = remote.projection_count(&CountReq::default());
            if matches!(code, 404 | 405) {
                assert!(result.unwrap().is_none())
            } else {
                assert!(result.is_err())
            }
            assert_eq!(
                worker.join().unwrap().len(),
                2,
                "no retries/fallback on HTTP {code}"
            );
        }
        for body in ["{}", "not-json", r#"{"declared":null}"#] {
            let (base, worker) =
                super::super::gateway_tests::server(vec![(200, "ok"), (200, body)]);
            let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
            assert!(remote.projection_count(&CountReq::default()).is_err());
            assert_eq!(worker.join().unwrap().len(), 2);
        }
    }

    #[test]
    fn projection_count_watermark_change_discards_read() {
        const LIST: &str = r#"{"declared":[{"name":"board","key":"https://seeds.local/ontology/status","columns":[]}] }"#;
        const BEFORE: &str = r#"{"tx":1,"total":1,"truncated":false,"multi_valued_rows":0,"groups":[{"value":"open","n":1}]}"#;
        const AFTER: &str = r#"{"tx":2,"total":1,"truncated":false,"multi_valued_rows":0,"groups":[{"value":"open","n":1}]}"#;
        let mut replies = vec![
            (200, "ok".to_string()),
            (200, LIST.into()),
            (200, BEFORE.into()),
            (200, r#"{"boolean":false}"#.into()),
        ];
        replies.extend(scope_responses(false));
        replies.push((200, AFTER.into()));
        let refs = replies
            .into_iter()
            .map(|(code, body)| (code, &*Box::leak(body.into_boxed_str())))
            .collect();
        let (base, worker) = super::super::gateway_tests::server(refs);
        let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
        assert!(remote
            .projection_count(&CountReq::default())
            .unwrap()
            .is_none());
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 11);
        assert!(requests[2].contains("urn:projection"));
        assert!(requests[3].contains("ASK"));
    }
}
