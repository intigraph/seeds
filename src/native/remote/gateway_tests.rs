//! Actual HTTP gateway recovery; updates must never be replayed.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};

fn server(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
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
fn gateway_outage_stops_after_six_attempts() {
    let (base, worker) = server(vec![(503, "still restarting"); 6]);
    let error = RemoteBackend::connect(&base, "urn:test", None, None, &[])
        .err()
        .expect("an exhausted outage is not healthy");
    assert!(error.message.contains("HTTP 503"));
    assert_eq!(worker.join().unwrap().len(), 6);
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
