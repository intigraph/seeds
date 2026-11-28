use super::*;
use base64::Engine as _;
use clap::Parser;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

const PROJECT: &str = "urn:test:project";
fn profile(base: &str, root: PathBuf) -> Profile {
    Profile {
        base: base.into(),
        project: PROJECT.into(),
        store_id: "store".into(),
        log_epoch: "epoch".into(),
        principal: "test-principal".into(),
        credential_id: "test-key".into(),
        denied_control_graph: "urn:test:denied".into(),
        state_dir: Some(root),
    }
}
fn cut(profile: &Profile, tx: u64) -> String {
    let mut members = serde_json::Map::new();
    for graph in profile.members() {
        members.insert(graph,json!({"store_id":"store","log_epoch":"epoch","local_tx":tx,"generation":1,"policy":"p1"}));
    }
    format!(
        "qcut1.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"version":1,"catalog_revision":1,"members":members}))
                .unwrap()
        )
    )
}
fn attempt() -> Attempt {
    let update = profile("", PathBuf::new())
        .members()
        .iter()
        .map(|g| format!("GRAPH <{g}> {{ <urn:test:s> <urn:test:p> <urn:test:o> }}"))
        .collect::<Vec<_>>()
        .join(" ");
    let update = format!("INSERT DATA {{ {update} }}");
    let form = format!(
        "update={}&actor=test&source=seeds",
        super::super::form_encode(&update)
    );
    Attempt {
        schema: ATTEMPT_CONTRACT.into(),
        write_id: "a".repeat(64),
        base: "http://localhost".into(),
        project: PROJECT.into(),
        store_id: "store".into(),
        log_epoch: "epoch".into(),
        principal: "test-principal".into(),
        credential_id: "test-key".into(),
        credential_sha256: crate::pendant::sha256(b"test-token"),
        intent_digest: "b".repeat(64),
        item_ids: vec!["test-one".into()],
        update,
        form: form.clone(),
        body_sha256: crate::pendant::sha256(form.as_bytes()),
        phase: "prepared".into(),
        expected: Expected {
            seeds: vec![],
            comments: vec![],
            delete_seeds: vec![],
            delete_comments: vec![],
            record: vec![],
        },
    }
}
#[test]
fn private_journal_publication_lock_corruption_and_terminal_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("attempts");
    let j = Journal::open(root.clone()).unwrap();
    let mut a = attempt();
    j.persist(&a, true).unwrap();
    let path = root.join(format!("{}.json", a.write_id));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(j.pending().unwrap().len(), 1);
    assert!(Journal::open(root.clone()).is_err());
    assert!(j.persist(&a, true).is_err());
    assert!(j.remove_terminal(&a).is_err());
    a.phase = "finished".into();
    j.persist(&a, false).unwrap();
    j.remove_terminal(&a).unwrap();
    assert!(j.pending().unwrap().is_empty());
    std::os::unix::fs::symlink(tmp.path().join("absent"), &path).unwrap();
    assert!(j.pending().is_err());
}
#[test]
fn envelope_binds_text_digest_and_private_profile() {
    let mut a = attempt();
    a.validate().unwrap();
    a.update.push(' ');
    assert!(a.validate().is_err());
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("profile");
    std::fs::write(&p, b"{}").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(journal::read_private(&p, 65536).is_err());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(journal::read_private(&p, 65536).unwrap(), b"{}");
}
struct Server {
    base: String,
    seen: Arc<Mutex<Vec<(String, String, String)>>>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        self.join.take().unwrap().join().unwrap();
    }
}
fn server(
    mut route: impl FnMut(&str, &str, &str) -> (u16, Value, Option<String>) + Send + 'static,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let seen = Arc::new(Mutex::new(vec![]));
    let calls = seen.clone();
    let join = std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            let mut stream = stream.unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut bytes = vec![];
            let mut one = [0; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                if stream.read_exact(&mut one).is_err() {
                    break;
                }
                bytes.push(one[0]);
                if bytes.len() > 65536 {
                    panic!("header bound");
                }
            }
            let headers = String::from_utf8(bytes).unwrap();
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let len = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut data = vec![0; len];
            stream.read_exact(&mut data).unwrap();
            let data = String::from_utf8(data).unwrap();
            calls
                .lock()
                .unwrap()
                .push((path.clone(), data.clone(), headers.clone()));
            let (code, value, cut) = route(&path, &data, &headers);
            let text = value.to_string();
            let extra = cut
                .map(|c| format!("quipu-composition-cut: {c}\r\n"))
                .unwrap_or_default();
            let response=format!("HTTP/1.1 {code} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{text}",text.len());
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Server {
        base,
        seen,
        stop,
        join: Some(join),
    }
}
fn capability() -> Value {
    json!({"routed_update_completion":{"operation":"/update","request_contract":REQUEST_CONTRACT,"status_contract":STATUS_CONTRACT,"enabled":true,"status":"/update/status","seal":"/update/seal"}})
}
fn routes(path: &str, body: &str, token: &str) -> Option<(u16, Value, Option<String>)> {
    match path {
        "/health" => Some((200, json!({"ok":true}), None)),
        "/version" => Some((200, capability(), None)),
        "/graph-sets/query" => {
            let input: Value = serde_json::from_str(body).unwrap();
            Some(if input["graphs"] == json!(["urn:test:denied"]) {
                (403, json!({"error":"composed read denied"}), None)
            } else {
                (
                    200,
                    if input["query"] == "ASK {}" {
                        json!({"boolean":true})
                    } else {
                        json!({"head":{"vars":[]},"results":{"bindings":[]}})
                    },
                    Some(token.into()),
                )
            })
        }
        _ => None,
    }
}
fn remote(s: &Server) -> RemoteBackend {
    RemoteBackend::connect(
        &s.base,
        PROJECT,
        Some("test-token".into()),
        None,
        &["127.0.0.1".into()],
    )
    .unwrap()
}
fn fixture(
    mut handler: impl FnMut(&str, &str) -> (u16, Value, Option<String>) + Send + 'static,
) -> (tempfile::TempDir, Server, Profile) {
    let tmp = tempfile::tempdir().unwrap();
    let p = profile("", tmp.path().join("attempts"));
    let token = cut(&p, 17);
    let s = server(move |path, body, _| {
        routes(path, body, &token).unwrap_or_else(|| handler(path, body))
    });
    let p = profile(&s.base, p.state_dir.unwrap());
    (tmp, s, p)
}
fn finished(id: &str, cut: &str) -> Value {
    json!({"contract":STATUS_CONTRACT,"write_id":id,"state":"finished","store_id":"store","log_epoch":"epoch","commit":{"tx_id":17},"receipt":{"asserted":1,"retracted":0,"owner":{"store_id":"store","log_epoch":"epoch","local_tx":17,"cut":cut}}})
}
#[test]
fn preflight_controls_and_composed_routing_never_use_legacy_query() {
    let (_tmp, s, p) = fixture(|_, _| panic!("unexpected endpoint"));
    let mut r = remote(&s);
    r.graph_sets = Some(Mode::open(&r, p).unwrap());
    r.select("SELECT ?s WHERE { ?s ?p ?o }", None).unwrap();
    assert!(r.send("/update", "application/json", "{}", true).is_err());
    assert!(r.select("SELECT ?s WHERE { ?s ?p ?o }", Some(1)).is_err());
    let calls = s.seen.lock().unwrap();
    assert!(!calls
        .iter()
        .any(|(p, _, _)| p == "/query" || p == "/update"));
    assert_eq!(
        calls
            .iter()
            .filter(|(p, _, _)| p == "/graph-sets/query")
            .count(),
        3
    );
}
#[test]
fn disabled_keeps_legacy_wire_without_preflight() {
    let s = server(|p, b, h| match p {
        "/health" => (200, json!({}), None),
        "/query" => {
            assert_eq!(b, "{\"query\":\"ASK {}\"}");
            assert!(!h.to_ascii_lowercase().contains("x-quipu-write-id"));
            (200, json!({"boolean":true}), None)
        }
        _ => panic!("unexpected endpoint {p}"),
    });
    let r = remote(&s);
    assert!(r.graph_sets.is_none());
    r.send(
        "/query",
        "application/json",
        "{\"query\":\"ASK {}\"}",
        false,
    )
    .unwrap();
    assert_eq!(s.seen.lock().unwrap().len(), 2);
}
#[test]
fn restart_after_send_same_create_recovers_original_without_second_send() {
    let p = profile("", PathBuf::from("/unused"));
    let token = cut(&p, 17);
    let original = "a".repeat(64);
    let receipt = finished(&original, &token);
    let (_tmp, s, p) = fixture(move |path, body| match path {
        "/update" => (200, json!({}), None),
        "/update/status" => {
            assert_eq!(
                serde_json::from_str::<Value>(body).unwrap()["write_id"],
                original
            );
            (200, receipt.clone(), None)
        }
        _ => panic!("unexpected {path}"),
    });
    let mut r = remote(&s);
    let mode = Mode::open(&r, p.clone()).unwrap();
    let cli =
        crate::native::cli::Cli::try_parse_from(["sd", "create", "same title", "--json"]).unwrap();
    let intent = intent::make(&cli, "tester", PROJECT).unwrap();
    let mut a = attempt();
    a.base = s.base.clone();
    a.intent_digest = intent.digest;
    mode.journal.persist(&a, true).unwrap();
    mode.send_attempt(&r, &a).unwrap();
    drop(mode); // simulated death after send, before recording a response
    r.graph_sets = Some(Mode::open(&r, p).unwrap());
    let outcome = r.graph_set_prepare(&cli, &ctx()).unwrap().unwrap();
    assert_eq!(outcome.code, 0);
    let value: Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(value["item"], "test-one");
    assert_eq!(value["owner"]["local_tx"], 17);
    assert_eq!(
        s.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _)| p == "/update")
            .count(),
        1
    );
    assert!(r
        .graph_sets
        .as_ref()
        .unwrap()
        .journal
        .pending()
        .unwrap()
        .is_empty());
}
fn ctx() -> crate::backend::Ctx {
    crate::backend::Ctx {
        actor: "tester".into(),
        now: "2026-01-01T00:00:00Z".into(),
        prefix: "test".into(),
        claims: Default::default(),
    }
}
#[test]
fn witness_only_same_intent_and_same_item_refuse_other_item_proceeds() {
    let (_tmp, s, p) = fixture(|path, body| {
        assert_eq!(path, "/update/status");
        let v: Value = serde_json::from_str(body).unwrap();
        (
            200,
            json!({"contract":STATUS_CONTRACT,"write_id":v["write_id"],"state":"committed","store_id":"store","log_epoch":"epoch","commit":{"tx_id":17}}),
            None,
        )
    });
    let mut r = remote(&s);
    let mode = Mode::open(&r, p).unwrap();
    let cli =
        crate::native::cli::Cli::try_parse_from(["sd", "update", "test-one", "--title", "one"])
            .unwrap();
    let mut a = attempt();
    a.base = s.base.clone();
    a.intent_digest = intent::make(&cli, "tester", PROJECT).unwrap().digest;
    mode.journal.persist(&a, true).unwrap();
    r.graph_sets = Some(mode);
    let e = r.graph_set_prepare(&cli, &ctx()).unwrap_err();
    assert_eq!(e.kind.exit_code(), 8);
    assert_eq!(e.details.unwrap()["state"], "committed_witness_only");
    let other = crate::native::cli::Cli::try_parse_from([
        "sd",
        "close",
        "test-one",
        "--reason",
        "different",
    ])
    .unwrap();
    assert_eq!(
        r.graph_set_prepare(&other, &ctx())
            .unwrap_err()
            .kind
            .exit_code(),
        8
    );
    let other =
        crate::native::cli::Cli::try_parse_from(["sd", "update", "test-two", "--title", "two"])
            .unwrap();
    assert!(r.graph_set_prepare(&other, &ctx()).unwrap().is_none());
    assert_eq!(
        r.graph_sets
            .as_ref()
            .unwrap()
            .journal
            .pending()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn preflight_refuses_disabled_capability_and_bad_credential_control() {
    for disabled in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let p = profile("", tmp.path().join("attempts"));
        let token = cut(&p, 17);
        let s = server(move |path, body, _| {
            if path == "/version" && disabled {
                let mut c = capability();
                c["routed_update_completion"]["enabled"] = json!(false);
                return (200, c, None);
            }
            if path == "/graph-sets/query" && body.contains("urn:test:denied") {
                return (200, json!({"boolean":true}), Some(token.clone()));
            }
            routes(path, body, &token).expect("no mutation may be sent")
        });
        let r = remote(&s);
        assert!(Mode::open(&r, profile(&s.base, p.state_dir.unwrap())).is_err());
        assert!(!s
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _, _)| p == "/update"));
    }
}
#[test]
fn cut_rejects_wrong_selection_lineage_noncanonical_and_behind_witness() {
    let (_tmp, s, p) = fixture(|_, _| panic!("unexpected"));
    let r = remote(&s);
    let mode = Mode::open(&r, p.clone()).unwrap();
    mode.check_cut(&cut(&p, 17)).unwrap();
    assert!(mode
        .check_cut_for(&cut(&p, 16), &p.members(), Some(17))
        .is_err());
    for change in ["selection", "epoch", "canonical"] {
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cut(&p, 17).strip_prefix("qcut1.").unwrap())
            .unwrap();
        let mut v: Value = serde_json::from_slice(&raw).unwrap();
        match change {
            "selection" => {
                let x = v["members"]
                    .as_object_mut()
                    .unwrap()
                    .remove(PROJECT)
                    .unwrap();
                v["members"]["urn:foreign"] = x;
            }
            "epoch" => v["members"][PROJECT]["log_epoch"] = json!("foreign"),
            _ => {}
        }
        let bytes = if change == "canonical" {
            serde_json::to_vec_pretty(&v).unwrap()
        } else {
            serde_json::to_vec(&v).unwrap()
        };
        let bad = format!(
            "qcut1.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        );
        assert!(mode.check_cut(&bad).is_err());
    }
}
#[test]
fn admitted_recovery_seals_before_new_intent_never_replays_old_bytes() {
    let (_tmp, s, p) = fixture(|path, body| {
        let v: Value = serde_json::from_str(body).unwrap();
        let state = match path {
            "/update/status" => "admitted",
            "/update/seal" => "sealed",
            _ => panic!("unexpected {path}"),
        };
        (
            200,
            json!({"contract":STATUS_CONTRACT,"write_id":v["write_id"],"state":state,"store_id":"store","log_epoch":"epoch"}),
            None,
        )
    });
    let mut r = remote(&s);
    let mode = Mode::open(&r, p).unwrap();
    let mut a = attempt();
    a.base = s.base.clone();
    mode.journal.persist(&a, true).unwrap();
    r.graph_sets = Some(mode);
    let cli =
        crate::native::cli::Cli::try_parse_from(["sd", "create", "different title", "--json"])
            .unwrap();
    assert!(r.graph_set_prepare(&cli, &ctx()).unwrap().is_none());
    let outcome = r
        .graph_set_finish(
            &cli,
            crate::native::Outcome {
                code: 0,
                stdout: "{\"id\":\"test-two\"}".into(),
                stderr: String::new(),
            },
        )
        .unwrap();
    let v: Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["recovered_prior"][0]["state"], "sealed");
    let calls = s.seen.lock().unwrap();
    let paths: Vec<_> = calls.iter().map(|(p, _, _)| p.as_str()).collect();
    assert!(paths.ends_with(&["/update/status", "/update/seal"]));
    assert!(!paths.contains(&"/update"));
}
#[test]
fn different_intent_reports_recovered_prior_and_sends_new_durable_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("attempts");
    let old = "a".repeat(64);
    let token = cut(&profile("", root.clone()), 17);
    let old_for_server = old.clone();
    let at_send = root.clone();
    let s = server(move |path, body, _| {
        if let Some(answer) = routes(path, body, &token) {
            return answer;
        }
        match path {
            "/update/status" => {
                let v: Value = serde_json::from_str(body).unwrap();
                (200, finished(v["write_id"].as_str().unwrap(), &token), None)
            }
            "/update" => {
                let files: Vec<_> = std::fs::read_dir(&at_send)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect();
                assert_eq!(files.len(), 1);
                let bytes = journal::read_private(&files[0], 4 * 1024 * 1024).unwrap();
                let pending: Attempt = serde_json::from_slice(&bytes).unwrap();
                assert_ne!(pending.write_id, old_for_server);
                assert_eq!(pending.form, body);
                (200, json!({}), None)
            }
            _ => panic!("unexpected {path}"),
        }
    });
    let mut r = remote(&s);
    let mode = Mode::open(&r, profile(&s.base, root)).unwrap();
    let mut a = attempt();
    a.base = s.base.clone();
    mode.journal.persist(&a, true).unwrap();
    r.graph_sets = Some(mode);
    let cli = crate::native::cli::Cli::try_parse_from([
        "sd", "update", "test-two", "--title", "new", "--json",
    ])
    .unwrap();
    assert!(r.graph_set_prepare(&cli, &ctx()).unwrap().is_none());
    let seed = crate::model::Seed {
        id: "test-two".into(),
        ..Default::default()
    };
    let batch = crate::backend::WriteBatch {
        seeds: vec![crate::backend::SeedWrite {
            seed,
            expected_revision: Some(1),
        }],
        ..Default::default()
    };
    let mode = r.graph_sets.as_ref().unwrap();
    assert_eq!(mode.commit(&r, &batch, &a.update, &a.form, &[]).unwrap(), 0);
    let o = r
        .graph_set_finish(
            &cli,
            crate::native::Outcome {
                code: 0,
                stdout: "{}".into(),
                stderr: String::new(),
            },
        )
        .unwrap();
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["recovered_prior"][0]["write_id"], old);
    assert_eq!(
        s.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _)| p == "/update")
            .count(),
        1
    );
}

#[test]
fn committed_requires_one_complete_current_composed_state_and_provenance_proof() {
    for defect in ["none", "wrong-marker", "behind", "truncated"] {
        let tmp = tempfile::tempdir().unwrap();
        let p = profile("", tmp.path().join("attempts"));
        let token = cut(&p, 17);
        let proof_token = cut(&p, if defect == "behind" { 16 } else { 17 });
        let provenance = super::super::provenance_graph(PROJECT);
        let s = server(move |path, body, _| {
            if path == "/graph-sets/query" {
                let v: Value = serde_json::from_str(body).unwrap();
                if v["query"] != "ASK {}" && v["graphs"] != json!(["urn:test:denied"]) {
                    let object = if defect == "wrong-marker" {
                        "urn:other"
                    } else {
                        "urn:operation"
                    };
                    return (
                        200,
                        json!({"head":{"vars":["g","s","p","o"]},"results":{"bindings":[{
                        "g":{"type":"uri","value":provenance},"s":{"type":"uri","value":"urn:marker"},
                        "p":{"type":"uri","value":"urn:predicate"},"o":{"type":"uri","value":object}
                    }]},"truncated":defect=="truncated"}),
                        Some(proof_token.clone()),
                    );
                }
            }
            if let Some(answer) = routes(path, body, &token) {
                return answer;
            }
            assert_eq!(path, "/update/status");
            let v: Value = serde_json::from_str(body).unwrap();
            (
                200,
                json!({"contract":STATUS_CONTRACT,"write_id":v["write_id"],"state":"committed","store_id":"store","log_epoch":"epoch","commit":{"tx_id":17}}),
                None,
            )
        });
        let mut r = remote(&s);
        let mode = Mode::open(&r, profile(&s.base, p.state_dir.unwrap())).unwrap();
        let cli = crate::native::cli::Cli::try_parse_from([
            "sd", "update", "test-one", "--title", "intent", "--json",
        ])
        .unwrap();
        let mut a = attempt();
        a.base = s.base.clone();
        a.intent_digest = intent::make(&cli, "tester", PROJECT).unwrap().digest;
        a.expected.delete_seeds.push(("test-one".into(), 1));
        a.expected.record.push((
            "urn:marker".into(),
            vec![(
                "urn:predicate".into(),
                crate::model::Obj::Iri("urn:operation".into()),
            )],
        ));
        mode.journal.persist(&a, true).unwrap();
        r.graph_sets = Some(mode);
        let result = r.graph_set_prepare(&cli, &ctx());
        if defect == "none" {
            let outcome = result.unwrap().unwrap();
            assert_eq!(outcome.code, 0);
            let record: Value = serde_json::from_str(&outcome.stdout).unwrap();
            assert_eq!(record["state"], "committed_proven");
            assert_eq!(record["custody_retained"], true);
        } else {
            let e = result.unwrap_err();
            assert_eq!(e.kind.exit_code(), 8, "{defect}");
            assert_eq!(e.details.unwrap()["state"], "committed_witness_only");
        }
        let pending = r.graph_sets.as_ref().unwrap().journal.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].phase, "committed");
        assert_eq!(
            s.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, b, _)| p == "/graph-sets/query" && !b.contains("ASK {}"))
                .count(),
            1
        );
    }
}

#[test]
fn declared_intent_normalizes_title_and_payload_aliases_and_ignores_clock() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("body");
    std::fs::write(&path, "payload").unwrap();
    let a = crate::native::cli::Cli::try_parse_from([
        "sd",
        "create",
        "title",
        "--description",
        "payload",
    ])
    .unwrap();
    let b = crate::native::cli::Cli::try_parse_from([
        "sd",
        "create",
        "--title",
        "title",
        "--description-file",
        path.to_str().unwrap(),
        "--silent",
    ])
    .unwrap();
    let a = intent::make(&a, "tester", PROJECT).unwrap();
    let b = intent::make(&b, "tester", PROJECT).unwrap();
    assert_eq!(a.digest, b.digest);
    assert!(hex_id(&a.digest));
    b.check_files().unwrap();
    std::fs::write(path, "different").unwrap();
    assert!(b.check_files().is_err());
    let mut data = json!({"tx":0,"agent_context":{"tx":42}});
    intent::clear_main_tx(&mut data);
    assert!(data["tx"].is_null());
    assert_eq!(data["agent_context"]["tx"], 42);
}
