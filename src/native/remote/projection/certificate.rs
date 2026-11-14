//! Same-snapshot server evidence for unfiltered status counts. Other column
//! types still need the positive scoped proof; a certificate never implies them.
use super::*;

fn clean(value: &Json, tx: u64, total: usize, identifier: bool) -> Result<bool> {
    let Some(cert) = value.get("certificate") else {
        return Ok(false);
    };
    if cert["scope"] != "graph" || cert["class"] != term::work_item() {
        return Ok(false);
    }
    if cert["tx"].as_u64() != Some(tx)
        || number(&cert["rows"])? != total
        || number(&cert["members"])? != total
    {
        return Ok(false);
    }
    for field in ["members_without_key", "rows_without_class", "key_not_plain"] {
        if number(&cert[field])? != 0 {
            return Ok(false);
        }
    }
    if identifier {
        for field in ["missing", "not_plain", "duplicated"] {
            if number(&cert["unique"]["identifier"][field])? != 0 {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

impl RemoteBackend {
    pub(super) fn certified_status_scope(
        &self,
        list: &Json,
        before: &Json,
        tx: u64,
        total: usize,
    ) -> Result<bool> {
        if !clean(before, tx, total, false)? {
            return Ok(false);
        }
        let definitions = list["declared"].as_array().ok_or_else(invalid)?;
        let items: Vec<_> = definitions
            .iter()
            .filter(|d| d["name"] == "board_items")
            .collect();
        let [items] = items.as_slice() else {
            return Ok(false);
        };
        if items["key"] != term::status() {
            return Ok(false);
        }
        let columns = items["columns"].as_array().ok_or_else(invalid)?;
        let ids: Vec<_> = columns
            .iter()
            .filter(|c| c["name"] == "identifier")
            .collect();
        let [id] = ids.as_slice() else {
            return Ok(false);
        };
        // A presence column does not carry the identifier's actual value.
        if id["predicate"] != term::identifier() || !id["equals"].is_null() {
            return Ok(false);
        }
        let Some(answer) = self.projection_read(json!({
            "name":"board_items", "graph":self.graph, "group_by":"key",
            "max_groups":ROW_CAP,
            "certificate":{"class":term::work_item(),"unique":["identifier"]}
        }))?
        else {
            return Ok(false);
        };
        let (items_tx, items_total) = aggregate(&answer)?;
        Ok(items_tx == tx
            && items_total == total
            && number(&answer["multi_valued_rows"])? == 0
            && clean(&answer, tx, total, true)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(total: usize) -> Json {
        json!({"tx":7,"total":total,"truncated":false,"multi_valued_rows":0,
            "groups":[{"value":"open","n":total}],
            "certificate":{"scope":"graph","class":term::work_item(),"tx":7,
                "rows":total,"members":total,"members_without_key":0,
                "rows_without_class":0,"key_not_plain":0,
                "unique":{"identifier":{"missing":0,"not_plain":0,"duplicated":0}}}})
    }

    #[test]
    fn certificate_requires_every_anomaly_and_snapshot_binding() {
        let valid = answer(120_000);
        assert!(clean(&valid, 7, 120_000, true).unwrap());
        assert!(!clean(&valid, 8, 120_000, true).unwrap());
        assert!(!clean(&valid, 7, 119_999, true).unwrap());
        assert!(!clean(&json!({}), 7, 0, false).unwrap());
        for field in ["members_without_key", "rows_without_class", "key_not_plain"] {
            let mut bad = valid.clone();
            bad["certificate"][field] = json!(1);
            assert!(!clean(&bad, 7, 120_000, true).unwrap(), "{field}");
            bad["certificate"][field] = Json::Null;
            assert!(clean(&bad, 7, 120_000, true).is_err(), "{field}");
        }
        for field in ["missing", "not_plain", "duplicated"] {
            let mut bad = valid.clone();
            bad["certificate"]["unique"]["identifier"][field] = json!(1);
            assert!(!clean(&bad, 7, 120_000, true).unwrap(), "{field}");
            bad["certificate"]["unique"]["identifier"][field] = json!(-1);
            assert!(clean(&bad, 7, 120_000, true).is_err(), "{field}");
        }
        for field in ["rows", "members", "tx"] {
            let mut bad = valid.clone();
            bad["certificate"][field] = json!(0);
            assert!(!clean(&bad, 7, 120_000, true).unwrap(), "{field}");
        }
        for (field, value) in [("scope", "all"), ("class", "urn:other")] {
            let mut bad = valid.clone();
            bad["certificate"][field] = json!(value);
            assert!(!clean(&bad, 7, 120_000, true).unwrap());
        }
    }

    #[test]
    fn certified_large_board_skips_subject_discovery_and_brackets_writes() {
        for items_tx in [7, 8] {
            let list = json!({"declared":[
                {"name":"board","key":term::status(),"columns":[]},
                {"name":"board_items","key":term::status(),"columns":[
                    {"name":"identifier","predicate":term::identifier()}]}]});
            let before = answer(120_000);
            let mut items = before.clone();
            items["tx"] = json!(items_tx);
            items["certificate"]["tx"] = json!(items_tx);
            // On a race, retain the existing scoped discovery fallback. Its
            // full LIMIT answer is incomplete, so the optimization declines.
            let fallback = json!({"truncated":true});
            let replies = vec![
                (200, "ok".to_string()),
                (200, list.to_string()),
                (200, before.to_string()),
                (200, r#"{"boolean":false}"#.into()),
                (200, items.to_string()),
                (
                    200,
                    if items_tx == 7 {
                        before.to_string()
                    } else {
                        fallback.to_string()
                    },
                ),
            ];
            let refs = replies
                .into_iter()
                .map(|(code, body)| (code, &*Box::leak(body.into_boxed_str())))
                .collect();
            let (base, worker) = super::super::super::gateway_tests::server(refs);
            let remote =
                RemoteBackend::connect(&base, "urn:certificate-project", None, None, &[]).unwrap();
            let result = remote.projection_count(&CountReq::default()).unwrap();
            if items_tx == 7 {
                assert_eq!(result.unwrap().total, 120_000);
            } else {
                assert!(result.is_none());
            }
            let requests = worker.join().unwrap();
            assert_eq!(requests.len(), 6);
            assert!(requests[2].contains("certificate"));
            assert!(requests[4].contains("identifier"));
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.contains("SELECT DISTINCT ?s"))
                    .count(),
                usize::from(items_tx != 7)
            );
        }
    }
    #[test]
    fn stats_status_totals_keep_closed_and_tombstones() {
        let list = json!({"declared":[
            {"name":"board","key":term::status(),"columns":[]},
            {"name":"board_items","key":term::status(),"columns":[
                {"name":"identifier","predicate":term::identifier()}]}]});
        let mut all = answer(12);
        all["groups"] = json!([
            {"value":"open","n":2}, {"value":"closed","n":9},
            {"value":"tombstone","n":1}]);
        let replies = vec![
            (200, "ok".to_string()),
            (200, list.to_string()),
            (200, all.to_string()),
            (200, r#"{"boolean":false}"#.into()),
            (200, all.to_string()),
            (200, all.to_string()),
        ];
        let refs = replies
            .into_iter()
            .map(|(code, body)| (code, &*Box::leak(body.into_boxed_str())))
            .collect();
        let (base, worker) = super::super::super::gateway_tests::server(refs);
        let remote =
            RemoteBackend::connect(&base, "urn:certificate-project", None, None, &[]).unwrap();
        assert_eq!(
            remote.projection_status_totals().unwrap().unwrap(),
            vec![
                ("closed".into(), 9),
                ("open".into(), 2),
                ("tombstone".into(), 1)
            ]
        );
        let requests = worker.join().unwrap();
        let body: Json = serde_json::from_str(requests[5].split("\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["key_in"], json!(["open", "closed", "tombstone"]));
        assert!(requests.iter().all(|r| !r.contains("SELECT DISTINCT ?s")));
    }
    #[test]
    fn stats_without_certificate_declines_without_subject_discovery() {
        let list = json!({"declared":[
            {"name":"board","key":term::status(),"columns":[]}]});
        let mut before = answer(120_000);
        before.as_object_mut().unwrap().remove("certificate");
        let replies = vec![
            (200, "ok".to_string()),
            (200, list.to_string()),
            (200, before.to_string()),
            (200, r#"{"boolean":false}"#.into()),
        ];
        let refs = replies
            .into_iter()
            .map(|(code, body)| (code, &*Box::leak(body.into_boxed_str())))
            .collect();
        let (base, worker) = super::super::super::gateway_tests::server(refs);
        let remote =
            RemoteBackend::connect(&base, "urn:certificate-project", None, None, &[]).unwrap();
        assert!(remote.projection_status_totals().unwrap().is_none());
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests.iter().all(|r| !r.contains("SELECT DISTINCT ?s")));
    }

    #[cfg(feature = "projection-contract-tests")]
    #[test]
    fn released_certificate_server_proves_real_counts_and_refuses_anomalies() {
        let bin = std::env::var("SEEDS_TEST_PROJECTIONS_SERVER")
            .expect("certificate contract requires SEEDS_TEST_PROJECTIONS_SERVER");
        let config = r#"
[quipu.shacl]
validate_on_write=false
[quipu.embedding]
auto_embed=false
[[quipu.projections]]
name="board"
key="https://seeds.local/ontology/status"
[[quipu.projections]]
name="board_items"
key="https://seeds.local/ontology/status"
[[quipu.projections.columns]]
name="identifier"
predicate="https://schema.org/identifier"
"#;
        let server = super::super::super::tests::quipu_server_config(&bin, Some(config));
        let graph = "urn:certificate-contract";
        let remote = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        let write = |triples: &str| {
            let query = format!("INSERT DATA {{ GRAPH <{graph}> {{ {triples} }} }}");
            remote
                .post(
                    "/update",
                    "application/x-www-form-urlencoded",
                    &format!("update={}", form_encode(&query)),
                    true,
                )
                .unwrap();
        };
        let seed = |subject: &str, id: &str, status: &str| {
            format!(
                "<{subject}> a <{}> ; <{}> \"{id}\" ; <{}> \"{status}\" .",
                term::work_item(),
                term::identifier(),
                term::status()
            )
        };
        write(
            &(seed("urn:a", "a", "open")
                + &seed("urn:b", "b", "closed")
                + &seed("urn:c", "c", "tombstone")),
        );
        let certified = || {
            let list = remote
                .projection_read(json!({"action":"list"}))
                .unwrap()
                .unwrap();
            let before = remote
                .projection_read(json!({"name":"board", "graph":graph,
                "group_by":"key", "certificate":{"class":term::work_item()}}))
                .unwrap()
                .unwrap();
            let (tx, total) = aggregate(&before).unwrap();
            remote
                .certified_status_scope(&list, &before, tx, total)
                .unwrap()
        };
        assert!(
            certified(),
            "fixture must serve the certificate API, never silently use legacy fallback"
        );
        for status in [
            None,
            Some("open"),
            Some("closed"),
            Some("tombstone"),
            Some("blocked"),
        ] {
            let req = CountReq {
                by: Some("status".into()),
                include_closed: true,
                filter: Filter {
                    status: status.map(str::to_string),
                    ..Filter::default()
                },
            };
            let projected = remote.projection_count(&req).unwrap().unwrap();
            let reference = remote.sparql_count_summary(&req).unwrap();
            assert_eq!(projected.total, reference.total);
            assert_eq!(projected.groups, reference.groups);
        }
        assert_eq!(
            remote.projection_status_totals().unwrap().unwrap(),
            vec![
                ("closed".into(), 1),
                ("open".into(), 1),
                ("tombstone".into(), 1)
            ]
        );
        write(&seed("urn:duplicate", "a", "open"));
        assert!(
            !certified(),
            "duplicate identifiers must decline certificate trust"
        );
    }
}
