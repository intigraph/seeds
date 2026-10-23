//! Optional current-board aggregate reads. Raw projection rows are not the
//! normalized Seeds model: prove compatibility on each call, never cache it.
use super::*;
use crate::engine::{Count, CountReq, Filter};

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

    pub(super) fn projection_count(&self, req: &CountReq) -> Result<Option<Count>> {
        // The API has no arbitrary column predicates or joins. Keep every
        // unsupported scope on the existing normalized SPARQL path.
        let mut filter = req.filter.clone();
        filter.status = None;
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
        if by != "status" {
            let predicate = match by {
                "priority" => term::priority(),
                "type" => term::issue_type(),
                _ => term::assigned_to(),
            };
            let columns = def["columns"].as_array().ok_or_else(invalid)?;
            if columns.iter().filter(|c| c["name"] == by).count() != 1
                || !columns
                    .iter()
                    .any(|c| c["name"] == by && c["predicate"] == predicate)
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
        if self.ephemeral_may_hold_facts()? {
            return Ok(None);
        }
        // Projection membership is status-based; Seeds membership additionally
        // needs Action + plain unique ID and defaults missing status to open.
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
        if by != "status" {
            let (predicate, valid) = match by {
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
            return Ok(None);
        }
        // Compare watermarks around ALL compatibility reads. A concurrent
        // commit (including an ephemeral write) discards the optimization.
        let group = if by == "status" || keys.is_empty() {
            "key"
        } else {
            by
        };
        let Some(after) = self.projection_read(request(group, &keys))? else {
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
        if projected_total != after_total {
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
        assert_eq!(parity(&remote, &scoped), CountSource::Sparql);
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
        eprintln!("COUNT_READER source=projection parity_cases={cases} semantic_fallbacks=6");
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
        let (base, worker) = super::super::gateway_tests::server(vec![
            (200, "ok"),
            (200, LIST),
            (200, BEFORE),
            (200, r#"{"boolean":false}"#),
            (200, r#"{"boolean":false}"#),
            (200, AFTER),
        ]);
        let remote = RemoteBackend::connect(&base, "urn:projection", None, None, &[]).unwrap();
        assert!(remote
            .projection_count(&CountReq::default())
            .unwrap()
            .is_none());
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 6);
        assert!(requests[2].contains("urn:projection"));
        assert!(requests[3].contains("ASK"));
    }
}
