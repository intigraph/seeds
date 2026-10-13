//! The built CLI crosses a real loopback transport; missing declarations stay missing.
#![cfg(feature = "native")]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used
)]
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

type Headers = BTreeMap<String, String>;

struct PrivateHome(std::path::PathBuf);
impl PrivateHome {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "seeds-read-attribution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for PrivateHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn probe(extra: &[(&str, &str)]) -> Vec<Headers> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let (stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("fixture listener: {e}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut headers = BTreeMap::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.trim_end().split_once(':') {
                    let key = key.to_ascii_lowercase();
                    if key == "content-length" {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                    // Never collect credentials or arbitrary headers.
                    if [
                        "x-quipu-client",
                        "x-quipu-agent",
                        "x-quipu-task",
                        "x-quipu-session",
                        "x-quipu-source",
                        "x-quipu-operation",
                    ]
                    .contains(&key.as_str())
                    {
                        headers.insert(key, value.trim().to_string());
                    }
                }
            }
            assert!(length <= 1024 * 1024);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(headers);
            let reply = if first.contains("/query ") {
                if String::from_utf8_lossy(&body).contains("ASK") {
                    r#"{"head":{},"boolean":false,"result":false}"#
                } else {
                    r#"{"head":{"vars":[]},"results":{"bindings":[]},"rows":[],"count":0,"truncated":false}"#
                }
            } else {
                r#"{"status":"ok","graphs":[]}"#
            };
            write!(reader.into_inner(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        }
    });
    let temp = PrivateHome::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_sd"));
    command
        .env_clear()
        .env("HOME", &temp.0)
        .env("XDG_CONFIG_HOME", &temp.0);
    command.envs(extra.iter().copied()).args([
        "--quipu",
        &url,
        "--graph",
        "https://example.test/project/control",
        "show",
        "fixture-1",
        "--json",
    ]);
    let result = command.output().unwrap();
    done.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert_eq!(
        result.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = seen.lock().unwrap().clone();
    assert_eq!(result.len(), 9, "health and eight unchanged query requests");
    for headers in &result {
        assert_eq!(headers["x-quipu-client"], "seeds");
        assert_eq!(headers["x-quipu-source"], "seeds");
        assert!(["health", "query"].contains(&headers["x-quipu-operation"].as_str()));
    }
    result
}

#[test]
fn unknown_and_stale_context_stay_unknown_on_real_requests() {
    for extra in [vec![], vec![("SHANTY_AGENT", "stale-crew")]] {
        for headers in probe(&extra) {
            assert!(!headers.contains_key("x-quipu-agent"));
            assert!(!headers.contains_key("x-quipu-task"));
        }
    }
}

#[test]
fn explicit_and_known_harness_context_cross_real_transport() {
    for headers in probe(&[
        ("QUIPU_AGENT", "declared-reader"),
        ("SHANTY_AGENT", "other-reader"),
        ("QUIPU_TASK", "test-work"),
        ("QUIPU_SESSION", "declared-session"),
    ]) {
        assert_eq!(headers["x-quipu-agent"], "declared-reader");
        assert_eq!(headers["x-quipu-task"], "test-work");
        assert_eq!(headers["x-quipu-session"], "declared-session");
    }
    for headers in probe(&[
        ("SHANTY_AGENT", "declared-crew"),
        ("CODEX_HOME", "declared-harness"),
        ("QUIPU_TASK", "test-work"),
    ]) {
        assert_eq!(headers["x-quipu-agent"], "declared-crew");
        assert_eq!(headers["x-quipu-task"], "test-work");
    }
}
