//! aegis-0aua: reading from a Quipu read replica says so, with the replica's
//! staleness, on stderr; stdout and its `--json` shape stay as they were.
#![cfg(feature = "native")]
// These drive the native `sd` binary, so they use the process and the
// filesystem the core is barred from (clippy.toml).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

const REPLICA_HEADERS: &str = "X-Quipu-Replica-Lag-Tx: unknown\r\n\
    X-Quipu-Replica-Source-Tx: 6\r\n\
    X-Quipu-Replica-Source-Time: 2026-10-25T09:48:39Z\r\n\
    X-Quipu-Replica-Confirmed-Age-S: 21\r\n";

/// A fake quipu answering every read with an empty result and every write with
/// the replica's refusal. `replica` adds the staleness headers a replica sends.
fn fake_quipu(replica: bool) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let (request, query) = read_request(&mut stream);
            let write = request.starts_with("POST /update") || request.starts_with("POST /knot");
            let (status, body) = if replica && write {
                (
                    "503 Service Unavailable",
                    r#"{"reason":"read_only_replica"}"#,
                )
            } else if query.contains("probe") {
                // The vocabulary/presence preflight: complete, nothing legacy.
                (
                    "200 OK",
                    r#"{"results":{"bindings":[{"probe":{"type":"literal","value":"complete"}}]}}"#,
                )
            } else {
                (
                    "200 OK",
                    r#"{"boolean":false,"graphs":[],"head":{"vars":[]},"results":{"bindings":[]}}"#,
                )
            };
            let headers = if replica { REPLICA_HEADERS } else { "" };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

/// The request line and body, after reading the whole request, so closing
/// the connection never resets it under the client.
fn read_request(stream: &mut std::net::TcpStream) -> (String, String) {
    let mut reader = std::io::BufReader::new(stream);
    let mut first = String::new();
    let _ = reader.read_line(&mut first);
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);
    (first, String::from_utf8_lossy(&body).into_owned())
}

fn sd(url: &str, args: &[&str]) -> Output {
    let home: PathBuf = std::env::temp_dir().join(format!(
        "sd-replica-notice-{}-{}",
        std::process::id(),
        args.join("-")
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_sd"))
        .args([
            "--quipu",
            url,
            "--graph",
            "https://example.org/project/test",
        ])
        .args(args)
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("SEEDS_QUIPU_URL")
        .env_remove("SEEDS_QUIPU_STORE")
        .env_remove("SEEDS_QUIPU_TOKEN")
        .env_remove("QUIPU_AUTH_TOKEN")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&home);
    out
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn a_read_from_a_replica_states_its_staleness_and_leaves_stdout_alone() {
    let primary = fake_quipu(false);
    let replica = fake_quipu(true);

    let before = sd(&primary, &["list"]);
    let after = sd(&replica, &["list"]);
    assert!(before.status.success(), "{}", text(&before.stderr));
    assert!(after.status.success(), "{}", text(&after.stderr));
    assert_eq!(after.stdout, before.stdout, "stdout is unchanged");
    assert!(
        !text(&before.stderr).contains("replica"),
        "no notice off a replica"
    );
    assert!(
        text(&after.stderr).contains(
            "sd: read from a Quipu read replica; writes are refused. Source tx 6 \
             (committed 2026-10-25T09:48:39Z), confirmed current 21 s ago, lag unknown tx."
        ),
        "{}",
        text(&after.stderr)
    );

    let before = sd(&primary, &["list", "--json"]);
    let after = sd(&replica, &["list", "--json"]);
    assert!(after.status.success(), "{}", text(&after.stderr));
    assert_eq!(after.stdout, before.stdout, "the --json shape is unchanged");
    let line = text(&after.stderr)
        .lines()
        .find(|l| l.starts_with("{\"replica\""))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no replica JSON line: {}", text(&after.stderr)));
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"replica": {"writes": "refused", "source_tx": 6,
            "source_time": "2026-10-25T09:48:39Z", "confirmed_age_s": 21, "lag_tx": null}})
    );
}

#[test]
fn a_refused_write_on_a_replica_names_the_replica() {
    let replica = fake_quipu(true);
    let out = sd(&replica, &["q", "a write the replica refuses"]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains("read from a Quipu read replica; writes are refused"),
        "{err}"
    );
}

#[test]
fn an_identity_refusal_from_a_replica_still_names_the_replica() {
    // tim's #199 review: failures after the connect but before the verb (the
    // identity read, the vocabulary check) dropped the notice.
    let replica = fake_quipu(true);
    let out = sd(&replica, &["identity"]);
    let err = text(&out.stderr);
    assert!(err.contains("read from a Quipu read replica"), "{err}");
}

#[test]
fn a_sync_to_a_replica_is_refused_before_anything_moves() {
    // tim's #199 SAME: the refusal must come before the local store is opened
    // or hydrated, so "nothing was read or written" is true locally too.
    let replica = fake_quipu(true);
    let home = std::env::temp_dir().join(format!("sd-replica-sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let run = |store: &std::path::Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sd"))
            .args(["--store", store.to_str().unwrap()])
            .args(args)
            .current_dir(&home)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env_remove("SEEDS_QUIPU_URL")
            .env_remove("SEEDS_QUIPU_STORE")
            .env_remove("SEEDS_QUIPU_TOKEN")
            .env_remove("QUIPU_AUTH_TOKEN")
            .output()
            .unwrap()
    };
    // A store that does not exist yet is not created.
    let fresh = home.join("fresh.db");
    let out = run(&fresh, &["sync", "--remote", &replica]);
    let err = text(&out.stderr);
    assert!(!out.status.success());
    assert!(
        err.contains("is a Quipu read replica; nothing was read or written"),
        "{err}"
    );
    assert!(!fresh.exists(), "the refusal created the local store");
    // An existing store is byte-identical afterwards.
    let held = home.join("held.db");
    assert!(run(&held, &["q", "a local seed"]).status.success());
    let before = std::fs::read(&held).unwrap();
    let out = run(&held, &["sync", "--remote", &replica]);
    assert!(!out.status.success());
    assert_eq!(
        std::fs::read(&held).unwrap(),
        before,
        "the refusal changed the local store"
    );
    let _ = std::fs::remove_dir_all(&home);
}
