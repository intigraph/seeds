//! Actual CLI wire controls: a summary read cannot fall back to aggregates.
#![cfg(feature = "native")]
// Native CLI fixtures use a local HTTP server and private filesystem paths.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

fn summary() -> Value {
    json!({"contract":"project-summary/v1","graph":"urn:example:project","summary":{
        "project_iri":"urn:example:project","ephemeral_iri":"urn:example:project#seeds-ephemeral",
        "coverage":{"project":1,"ephemeral":2,"store_id":"urn:example:store","schema_id":"project-summary-coverage-v5","generation":3},
        "evaluated_at":"2026-10-17T00:00:00.000000000Z","valid_until":null,
        "counts":{"maintenance":"transactional-static-counts","total":1,"tombstones":0,"comments":2,
            "by_status":{"open":1},"by_kind":{"task":1},"by_priority":{"2":1},"by_assignee":{"(unassigned)":1},"by_label":{"(no labels)":1},"dynamic_statistics":null,"semantic_transaction":null}
    }})
}
fn probe(body: Value, code: u16, exact: bool, success: bool) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (halt, seen) = (stop.clone(), requests.clone());
    let worker = std::thread::spawn(move || {
        while !halt.load(Ordering::Relaxed) {
            let (stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let path = first.split_ascii_whitespace().nth(1).unwrap().to_string();
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
            let text = String::from_utf8(bytes).unwrap();
            seen.lock().unwrap().push((path.clone(), text.clone()));
            let (status, answer) = match path.as_str() {
                "/health" => (200, json!({"status":"ok"})),
                "/query" if text.contains("SELECT ?probe WHERE") => (
                    200,
                    json!({"results":{"bindings":[{"probe":{"type":"literal","value":"complete"}}]}}),
                ),
                "/project/summary" => (code, body.clone()),
                _ => (
                    410,
                    json!({"error":"aggregate and fallback reads forbidden"}),
                ),
            };
            let answer = answer.to_string();
            write!(reader.get_mut(),"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",answer.len()).unwrap();
        }
    });
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let path = std::env::temp_dir().join(format!(
        "seeds-maintained-info-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    let cwd = Temp(path);
    std::fs::create_dir_all(cwd.0.join("home/.config")).unwrap();
    std::fs::create_dir(cwd.0.join("work")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_sd"));
    command
        .env_clear()
        .env("HOME", cwd.0.join("home"))
        .env("XDG_CONFIG_HOME", cwd.0.join("home/.config"))
        .env("SEEDS_ACTOR", "tester")
        .current_dir(cwd.0.join("work"))
        .args([
            "--quipu",
            &url,
            "--graph",
            "urn:example:project",
            "info",
            "--json",
        ]);
    if exact {
        command.arg("--exact-comments");
    }
    let output = command.output().unwrap();
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(requests[0].0, "/health");
    assert_eq!(requests[1].0, "/query");
    assert!(requests[1].1.contains("SELECT ?probe WHERE"));
    assert_eq!(
        requests[2],
        (
            "/project/summary".into(),
            r#"{"graph":"urn:example:project"}"#.into()
        )
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn maintained_info_static_zero_positive_comments_and_tombstone_contract() {
    let output = probe(summary(), 200, false, true);
    assert_eq!(output["issue_count"], 1);
    assert!(output["comment_count"].is_null() && output["tx"].is_null());
    assert_eq!(output["comment_count_status"], "not_computed");
    assert_eq!(output["summary_metadata"]["generation"], 3);
    let mut body = summary();
    body["summary"]["counts"]["tombstones"] = json!(2);
    let output = probe(body, 200, true, true);
    assert_eq!(output["issue_count"], 3);
    assert_eq!(output["comment_count"], 2);
    assert_eq!(output["comment_count_status"], "exact");
    let mut body = summary();
    let counts = &mut body["summary"]["counts"];
    counts["total"] = json!(0);
    for field in [
        "by_status",
        "by_kind",
        "by_priority",
        "by_assignee",
        "by_label",
    ] {
        counts[field] = json!({});
    }
    assert_eq!(probe(body, 200, false, true)["issue_count"], 0);
}

#[test]
fn maintained_info_refuses_unavailable_wrong_scope_schema_bounds_and_partial_data() {
    for code in [404, 409, 408] {
        probe(json!({"error":"unavailable"}), code, false, false);
    }
    let mut cases = Vec::new();
    for field in ["project_iri", "ephemeral_iri"] {
        let mut body = summary();
        body["summary"][field] = json!("urn:example:wrong-profile");
        cases.push(body);
        let mut body = summary();
        body["summary"].as_object_mut().unwrap().remove(field);
        cases.push(body);
    }
    for (field, value) in [
        ("contract", json!("unknown")),
        ("graph", json!("urn:example:other")),
    ] {
        let mut body = summary();
        body[field] = value;
        cases.push(body);
    }
    for (field, value) in [
        ("schema_id", json!("unknown")),
        ("generation", json!(-1)),
        ("ephemeral", json!(1)),
    ] {
        let mut body = summary();
        body["summary"]["coverage"][field] = value;
        cases.push(body);
    }
    let mut body = summary();
    body["summary"]["evaluated_at"] = json!("not-a-clock");
    cases.push(body);
    let mut body = summary();
    body["summary"]["valid_until"] = body["summary"]["evaluated_at"].clone();
    cases.push(body);
    let mut body = summary();
    body["summary"]["counts"]["by_status"]["open"] = json!(2);
    cases.push(body);
    let mut body = summary();
    body["summary"]["counts"]["semantic_transaction"] = json!(0);
    cases.push(body);
    let mut body = summary();
    body["summary"]["counts"]["dynamic_statistics"] = json!({});
    cases.push(body);
    let mut body = summary();
    body["summary"]["counts"]
        .as_object_mut()
        .unwrap()
        .remove("semantic_transaction");
    cases.push(body);
    let mut body = summary();
    body["padding"] = json!("x".repeat(65536));
    cases.push(body);
    for body in cases {
        probe(body, 200, false, false);
    }
}
