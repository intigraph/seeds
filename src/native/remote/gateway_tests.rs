//! Actual HTTP gateway recovery; updates must never be replayed.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};

pub(super) fn server(
    replies: Vec<(u16, &'static str)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (code, body) in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            requests.push(format!("{first}{}", String::from_utf8(bytes).unwrap()));
            write!(
                reader.get_mut(),
                "HTTP/1.1 {code} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    (base, worker)
}

#[test]
fn gateway_health_and_read_recover_but_write_is_unknown_without_replay() {
    let (base, worker) = server(vec![
        (503, "restart"),
        (200, "ok"),
        (502, "restart"),
        (200, "read-ok"),
        (502, "unknown-write"),
    ]);
    let b = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
    assert_eq!(
        b.post("/query", "application/json", "read-body", false)
            .unwrap(),
        "read-ok"
    );
    let (unknown, error) = b
        .send("/update", "application/sparql-update", "write-body", true)
        .unwrap_err();
    assert!(unknown);
    assert!(error.message.contains("HTTP 502"));
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[2], requests[3]);
    assert!(requests[4].contains("write-body"));
}

#[test]
fn gateway_does_not_retry_408_or_refusals() {
    for code in [400, 401, 403, 408, 500] {
        let (base, worker) = server(vec![(200, "ok"), (code, "refused")]);
        let b = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
        assert!(b
            .post("/query", "application/json", "read-body", false)
            .is_err());
        assert_eq!(worker.join().unwrap().len(), 2);
    }
}

#[test]
fn gateway_outage_stops_after_five_attempts() {
    let (base, worker) = server(vec![(503, "still restarting"); 5]);
    let error = RemoteBackend::connect(&base, "urn:test", None, None, &[])
        .err()
        .expect("an exhausted outage is not healthy");
    assert!(error.message.contains("HTTP 503"));
    assert_eq!(worker.join().unwrap().len(), 5);
}

#[test]
fn unordered_subject_ranges_are_complete_and_duplicate_pages_progress() {
    const FIRST: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:m"}},{"s":{"type":"uri","value":"urn:a"}},{"s":{"type":"uri","value":"urn:m"}}]}}"#;
    const LOWER: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:c"}},{"s":{"type":"uri","value":"urn:a"}},{"s":{"type":"uri","value":"urn:b"}}]}}"#;
    const A: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:a"}}]}}"#;
    const C: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:c"}}]}}"#;
    const Z: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:z"}}]}}"#;
    let (base, worker) = server(vec![
        (200, "ok"),
        (200, FIRST),
        (200, LOWER),
        (200, A),
        (200, C),
        (200, Z),
    ]);
    let mut backend = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
    backend.subject_page = 3;
    assert_eq!(
        backend
            .subjects_where("urn:test", "?s ?p ?o", None)
            .unwrap(),
        vec!["urn:a", "urn:b", "urn:c", "urn:m", "urn:z"]
    );
    let requests = worker.join().unwrap();
    assert!(requests.iter().all(|request| !request.contains("ORDER BY")));
    assert!(requests[3].contains("FILTER(STR(?s) <"));
    assert!(requests[4].contains("FILTER(STR(?s) >"));
}

#[test]
fn repeated_out_of_range_subject_refuses_instead_of_looping() {
    const SAME: &str = r#"{"results":{"bindings":[{"s":{"type":"uri","value":"urn:m"}},{"s":{"type":"uri","value":"urn:m"}}]}}"#;
    let (base, worker) = server(vec![(200, "ok"), (200, SAME), (200, SAME)]);
    let mut backend = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
    backend.subject_page = 2;
    let error = backend
        .subjects_where("urn:test", "?s ?p ?o", None)
        .unwrap_err();
    assert!(error.message.contains("outside its requested range"));
    assert_eq!(worker.join().unwrap().len(), 3);
}

#[test]
fn invalid_label_aggregate_refuses_without_a_snapshot_fallback() {
    for (body, expected) in [
        (
            r#"{"results":{"bindings":[]},"truncated":true}"#,
            "truncated",
        ),
        (
            r#"{"results":{"bindings":[{"label":{"type":"literal","value":"alpha"}}]}}"#,
            "invalid label count",
        ),
        (
            r#"{"results":{"bindings":[{"label":{"type":"literal","value":"alpha"},"n":{"type":"literal","value":"-1"}}]}}"#,
            "invalid label count",
        ),
        (
            r#"{"results":{"bindings":[{"n":{"type":"literal","value":"1"}}]}}"#,
            "invalid label",
        ),
    ] {
        let (base, worker) = server(vec![(200, "ok"), (200, body)]);
        let backend = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
        let error = crate::engine::labels(&backend, None, None).unwrap_err();
        assert!(error.message.contains(expected), "{error:?}");
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("GROUP BY ?label"));
        assert!(!requests[1].contains("ORDER BY"));
    }
}

#[test]
fn gateway_graph_and_history_reads_recover_with_metadata_validation_intact() {
    let (base, worker) = server(vec![
        (200, "ok"),
        (503, "restart"),
        (200, r#"{"graphs":[{"iri":"urn:test"}]}"#),
        (502, "restart"),
        (200, r#"{"transactions":[{"id":19}],"count":1}"#),
    ]);
    let backend = RemoteBackend::connect(&base, "urn:test", None, None, &[]).unwrap();
    assert!(backend.graph_iris().unwrap().contains("urn:test"));
    assert_eq!(
        backend.history_transaction_at_or_after(17).unwrap(),
        Some(19)
    );
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[1], requests[2]);
    assert_eq!(requests[3], requests[4]);
    assert!(requests[1].starts_with("GET /graphs "));
    assert!(requests[3].starts_with("GET /transactions?since=16&limit=1 "));
}

#[test]
fn owner_patterns_keep_defaults_without_a_separate_presence_request() {
    let (base, worker) = server(vec![(200, "ok")]);
    let backend = RemoteBackend::connect(&base, "urn:board", None, None, &[]).unwrap();
    for statuses in [
        None,
        Some(["closed".into()].into()),
        Some(["open".into()].into()),
    ] {
        let query = crate::backend::SeedQuery {
            assignee: Some("reader".into()),
            statuses,
            ..Default::default()
        };
        for graph in ["urn:board", "urn:other"] {
            let actual = backend.owner_seed_query_pattern(graph, &query).unwrap();
            assert_eq!(actual, vocab::seed_query_pattern_var(&query, "?s"));
        }
    }
    assert_eq!(
        worker.join().unwrap().len(),
        1,
        "health only; no race-prone presence read"
    );
}

#[test]
fn external_status_changes_never_hide_default_open_items() {
    use quipu::{Datum, Op, Store, Value};
    let (base, worker) = server(vec![(200, "ok")]);
    let backend = RemoteBackend::connect(&base, "urn:mutable", None, None, &[]).unwrap();
    let query = crate::backend::SeedQuery {
        assignee: Some("reader".into()),
        statuses: Some(["open".into()].into()),
        ..Default::default()
    };
    // First read observes no defaults; a different writer may mutate next.
    backend
        .owner_seed_query_pattern("urn:mutable", &query)
        .unwrap();
    worker.join().unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let graph = store.graph_create("urn:mutable").unwrap();
    let entity = store.intern("urn:seed:external").unwrap();
    let ty = store.intern(vocab::RDF_TYPE).unwrap();
    let class = store.intern(&term::work_item()).unwrap();
    let owner = store.intern(&term::assigned_to()).unwrap();
    let status = store.intern(&term::status()).unwrap();
    let datum = |attribute, value, op| Datum {
        entity,
        attribute,
        value,
        op,
        valid_from: "2026-01-01T00:00:00Z".into(),
        valid_to: None,
    };
    let apply = |store: &mut Store, facts: Vec<Datum>| {
        store
            .transact_to_graph(&facts, "2026-01-01T00:00:00Z", None, None, graph)
            .unwrap();
    };
    apply(
        &mut store,
        vec![
            datum(ty, Value::Ref(class), Op::Assert),
            datum(owner, Value::Str("reader".into()), Op::Assert),
        ],
    );
    let check = |store: &Store, expected: usize, stage: &str| {
        let actual = backend
            .owner_seed_query_pattern("urn:mutable", &query)
            .unwrap()
            .unwrap();
        let conservative = vocab::seed_query_pattern_var(&query, "?s").unwrap();
        let discover = |pattern: &str| {
            quipu::sparql::query(
                store,
                &format!("SELECT DISTINCT ?s WHERE {{ GRAPH <urn:mutable> {{ {pattern} }} }}"),
            )
            .unwrap()
            .rows()
            .to_vec()
        };
        let control = discover(&conservative);
        assert_eq!(
            control.len(),
            expected,
            "conservative positive/negative: {stage}"
        );
        assert_eq!(
            discover(&actual),
            control,
            "external change hidden: {stage}"
        );
    };
    check(&store, 1, "external addition without status");
    apply(
        &mut store,
        vec![datum(status, Value::Str("closed".into()), Op::Assert)],
    );
    check(&store, 0, "canonical closed is excluded");
    apply(
        &mut store,
        vec![datum(status, Value::Str("closed".into()), Op::Retract)],
    );
    check(&store, 1, "external status removal");
    apply(&mut store, vec![datum(status, Value::Int(7), Op::Assert)]);
    check(&store, 1, "external irregular status");
}

#[test]
fn signed_identity_checks_binding_and_never_sends_declared_actor() {
    for mutation in [
        "valid",
        "wrong_key",
        "wrong_session",
        "unkeyed",
        "wrong_class",
        "empty_principal",
    ] {
        let signer = crate::native::attest::Signer::from_seed(
            [47; 32],
            "synthetic-session",
            "urn:crew:introducer",
        )
        .unwrap();
        let mut value = json!({"authenticated":true,"auth_class":"attested_session",
            "principal":"synthetic-owner","key_id":signer.key_id(),"session":signer.session});
        match mutation {
            "wrong_key" => value["key_id"] = json!("sha256:wrong"),
            "wrong_session" => value["session"] = json!("other-session"),
            "unkeyed" => value["authenticated"] = json!(false),
            "wrong_class" => value["auth_class"] = json!("named_bearer"),
            "empty_principal" => value["principal"] = json!(""),
            _ => (),
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut captured = Vec::new();
            for body in ["ok".to_string(), value.to_string()] {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    headers.push_str(&line);
                }
                let mut payload = vec![0; length];
                reader.read_exact(&mut payload).unwrap();
                captured.push(headers.to_ascii_lowercase());
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            captured
        });
        let backend =
            RemoteBackend::connect(&base, "urn:fixture", None, Some(signer), &[]).unwrap();
        let result = backend.authenticated_identity();
        assert_eq!(result.is_ok(), mutation == "valid", "{mutation}");
        let requests = worker.join().unwrap();
        assert!(requests[1].starts_with("post /identity "));
        for request in requests {
            assert!(request.contains("x-quipu-attestation:"));
            assert!(!request.contains("x-quipu-agent:"));
            assert!(!request.contains("x-quipu-session:"));
            assert!(!request.contains("authorization:"));
        }
    }
}
