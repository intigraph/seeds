#![cfg(feature = "native")]
// Native CLI fixtures require process/filesystem APIs forbidden to the core.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Instant, SystemTime};

const GRAPH: &str = "urn:fixture:prime-board";
fn sd(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sd"))
        .env_clear()
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("USER", "fixture-agent")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn value(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn snapshot(root: &Path) -> Vec<(PathBuf, SystemTime, Vec<u8>)> {
    fn visit(root: &Path, p: &Path, out: &mut Vec<(PathBuf, SystemTime, Vec<u8>)>) {
        let m = std::fs::symlink_metadata(p).unwrap();
        out.push((
            p.strip_prefix(root).unwrap().to_owned(),
            m.modified().unwrap(),
            if m.is_file() {
                std::fs::read(p).unwrap()
            } else {
                vec![]
            },
        ));
        if m.is_dir() {
            for e in std::fs::read_dir(p).unwrap() {
                visit(root, &e.unwrap().path(), out);
            }
        }
    }
    let mut out = vec![];
    visit(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn static_and_absent_board_contract_never_create_paths() {
    let root = tempfile::tempdir().unwrap();
    let before = snapshot(root.path());
    for offline in [false, true] {
        for brief in [false, true] {
            let mut args = vec!["prime", "--json"];
            if offline {
                args.push("--static");
            }
            if brief {
                args.push("--brief");
            }
            let v = value(sd(root.path(), &args));
            assert_eq!(v["schema"], "stack-prime/1");
            assert_eq!(v["tool"], "sd");
            assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
            assert_eq!(v["primer"].get("text").is_none(), brief);
            assert!(v["primer"]["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Agent workflow"));
            assert_eq!(
                v["live"]["status"],
                if offline { "skipped" } else { "absent" }
            );
            assert_eq!(v["status"], if offline { "ok" } else { "degraded" });
            assert!(v["live"]["elapsed_ms"].as_u64().unwrap() < 1000);
            if !offline {
                assert!(v["live"]["summary"]["ready_count"].is_null());
            }
        }
        let args = if offline {
            vec!["prime", "--brief", "--static"]
        } else {
            vec!["prime", "--brief"]
        };
        let out = sd(root.path(), &args);
        assert!(out.status.success());
        let text = String::from_utf8(out.stdout).unwrap();
        assert!((5..=10).contains(&text.lines().count()), "{text}");
        assert!(text.starts_with(concat!("sd ", env!("CARGO_PKG_VERSION"), ":")));
        assert!(text
            .trim_end()
            .ends_with("Run `sd prime` for the full primer."));
    }
    assert_eq!(snapshot(root.path()), before);
    assert!(!root.path().join(".seeds").exists());
}

#[test]
fn local_plate_and_ready_reuse_engine_definitions_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("store ?#% ü.db");
    let base = ["--store", db.to_str().unwrap(), "--graph", GRAPH, "--json"];
    let create = |tail: &[&str]| -> Value {
        let mut args = base.to_vec();
        args.extend_from_slice(tail);
        value(sd(root.path(), &args))
    };
    create(&["create", "First", "--assignee", "fixture-agent"]);
    let blocker = create(&["create", "Blocker", "--assignee", "somebody-else"]);
    create(&[
        "create",
        "Dependent",
        "--assignee",
        "fixture-agent",
        "--deps",
        blocker["id"].as_str().unwrap(),
    ]);
    create(&[
        "create",
        "Ephemeral",
        "--assignee",
        "fixture-agent",
        "--ephemeral",
    ]);
    let before = snapshot(root.path());
    let mut args = base.to_vec();
    args.push("prime");
    let v = value(sd(root.path(), &args));
    assert_eq!(v["live"]["status"], "ok", "{v}");
    let summary = &v["live"]["summary"];
    assert_eq!(summary["plate_total"], 3);
    assert_eq!(summary["plate"].as_array().unwrap().len(), 3);
    assert_eq!(
        summary["ready_count"], 2,
        "dependent and ephemeral are not ready: {v}"
    );
    assert_eq!(summary["actor"], "fixture-agent");
    assert_eq!(summary["graph"], GRAPH);
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn sections_bad_flags_and_static_ignore_invalid_configuration() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/seeds")).unwrap();
    std::fs::write(root.path().join("config/seeds/config.toml"), "not = [valid").unwrap();
    let out = sd(root.path(), &["prime", "--static", "--section", "mcp"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("## MCP tools"));
    for args in [
        &["prime", "--section", "nope"][..],
        &["prime", "--unknown"][..],
        &["prime", "--section"][..],
    ] {
        assert_eq!(sd(root.path(), args).status.code(), Some(2));
    }
    let v = value(sd(root.path(), &["prime", "--json"]));
    assert_eq!(v["live"]["status"], "unreadable");
    let bad = sd(root.path(), &["prime", "--json", "--section", "nope"]);
    assert_eq!(bad.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(body["error"]["code"], "USAGE");
}

#[test]
fn unreachable_server_never_falls_back_to_a_local_store() {
    let root = tempfile::tempdir().unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    drop(socket);
    let v = value(sd(
        root.path(),
        &["--quipu", &url, "--graph", GRAPH, "prime", "--json"],
    ));
    assert_eq!(v["live"]["status"], "unreachable", "{v}");
    assert_eq!(v["live"]["summary"]["reachable"], false);
    assert!(!root.path().join(".seeds").exists());
}

#[test]
fn slow_server_is_partial_under_one_second_and_worker_is_reaped() {
    use std::io::Read;
    let root = tempfile::tempdir().unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    socket.set_nonblocking(true).unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed().as_secs() < 3 {
            match socket.accept() {
                Ok((mut conn, _)) => {
                    conn.set_read_timeout(Some(std::time::Duration::from_secs(1)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        assert_eq!(conn.read(&mut byte).unwrap(), 1);
                        request.push(byte[0]);
                        assert!(request.len() < 4096);
                    }
                    send.send(String::from_utf8(request).unwrap()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(900));
                    let mut byte = [0];
                    assert_eq!(
                        conn.read(&mut byte).unwrap(),
                        0,
                        "deadline must close the worker connection"
                    );
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let start = Instant::now();
    let v = value(sd(
        root.path(),
        &["--quipu", &url, "--graph", GRAPH, "prime", "--json"],
    ));
    assert!(start.elapsed().as_millis() < 1000, "{v}");
    assert_eq!(v["live"]["status"], "partial");
    assert!(v["live"]["summary"]["ready_count"].is_null());
    assert!(recv
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap()
        .starts_with("GET /health"));
    thread.join().unwrap();
    assert!(!root.path().join(".seeds").exists());
}

#[test]
fn summary_never_echoes_secrets_from_configuration_errors() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/seeds")).unwrap();
    std::fs::write(root.path().join("config/seeds/config.toml"), "[quipu]\nurl='https://example.invalid'\nstore='local.db'\ntoken='synthetic-private-token'\n").unwrap();
    let out = sd(root.path(), &["prime", "--json"]);
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("synthetic-private-token"));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["live"]["status"], "unreadable");
}

#[test]
fn healthy_remote_summary_uses_only_read_endpoints_and_the_configured_graph() {
    use std::io::{Read, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let done = stop.clone();
    let seen = requests.clone();
    let server = std::thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut conn, _)) => {
                    conn.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut b = [0];
                        if conn.read(&mut b).unwrap() == 0 {
                            break;
                        }
                        header.push(b[0]);
                        assert!(header.len() < 16384);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let n = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    assert!(n < 65536);
                    let mut body = vec![0; n];
                    conn.read_exact(&mut body).unwrap();
                    let request = header.lines().next().unwrap().to_owned();
                    let response = if request.starts_with("GET /health ") {
                        serde_json::json!({"ok":true})
                    } else {
                        assert!(request.starts_with("POST /query "), "{request}");
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        let query = body["query"].as_str().unwrap();
                        assert!(query.contains(GRAPH), "{query}");
                        let bindings = if query.starts_with("SELECT ?probe") {
                            serde_json::json!([{"probe":{"type":"literal","value":"complete"}}])
                        } else if query.contains("COUNT(") {
                            serde_json::json!([{"n":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":"0"}}])
                        } else {
                            serde_json::json!([])
                        };
                        if query.starts_with("ASK ") {
                            serde_json::json!({"boolean":false})
                        } else {
                            serde_json::json!({"head":{"vars":[]},"results":{"bindings":bindings},"truncated":false})
                        }
                    };
                    seen.lock().unwrap().push(request);
                    let body = response.to_string();
                    let _=write!(conn,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(2))
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let out = sd(
        root.path(),
        &["--quipu", &url, "--graph", GRAPH, "prime", "--json"],
    );
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let v = value(out);
    assert_eq!(v["live"]["status"], "ok", "{v}");
    assert_eq!(v["live"]["summary"]["reachable"], true);
    assert_eq!(v["live"]["summary"]["ready_count"], 0);
    assert_eq!(v["live"]["summary"]["plate_total"], 0);
    assert!(requests.lock().unwrap().len() > 2);
    assert!(!root.path().join(".seeds").exists());
}
