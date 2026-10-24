//! The crew topology: several declared actors share one host signing key.
#![cfg(feature = "native")]
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use base64::Engine as _;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[test]
fn shared_host_key_observes_each_actor_without_replacing_it_and_enforce_refuses() {
    let root = std::env::temp_dir().join(format!("seeds-host-actors-{}", std::process::id()));
    assert!(!root.exists(), "retained fixture already exists");
    std::fs::create_dir_all(root.join("home/.config/seeds")).unwrap();
    std::fs::create_dir_all(root.join("work")).unwrap();
    let key = root.join("host.key");
    // Public synthetic fixture seed, never an installed credential.
    std::fs::write(&key, "2f".repeat(32)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let signer =
        seeds::native::attest::Signer::from_seed([47; 32], "host-session", "urn:test:lead")
            .unwrap();
    let identity = json!({"authenticated":true,"auth_class":"attested_session", "principal":"host-principal", "key_id":signer.key_id(),"session":"host-session"});
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    // URL parsing maps this IPv4 spelling to the private loopback listener,
    // but it is deliberately NOT in the client's conservative HTTP exception
    // list. Thus the pre-profile signed-only transport gate really is tested,
    // without DNS, a LAN listener, or a configured host allowance.
    let url = format!("http://127.1:{}", listener.local_addr().unwrap().port());
    assert!(!seeds::native::remote::secure_enough(&url));
    assert_eq!(url::Url::parse(&url).unwrap().host_str(), Some("127.0.0.1"));
    let allow_identity = Arc::new(AtomicBool::new(true));
    let served_identity = allow_identity.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let worker_stop = stop.clone();
    let captured = requests.clone();
    let worker = std::thread::spawn(move || {
        while !worker_stop.load(Ordering::Relaxed) {
            let (stream, _) = match listener.accept() {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("accept: {e}"),
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
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
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let query: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
            let query = query["query"].as_str().unwrap_or("");
            let body = match path {
                "/health" => json!({"ok":true}),
                "/identity" if served_identity.load(Ordering::Relaxed) => identity.clone(),
                "/identity" => json!({"authenticated":false}),
                "/graphs" => json!({"graphs":[]}),
                "/graph/create" => json!({"ok":true}),
                "/query" if query.starts_with("SELECT ?probe") => {
                    json!({"results":{"bindings":[{"probe":{"type":"literal","value":"complete"}}]}})
                }
                "/query" if query.starts_with("ASK") => json!({"boolean":false}),
                "/query" if query.contains("COUNT(") => json!({"results":{"bindings":[{"n":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":"0"}}]}}),
                "/query" => json!({"results":{"bindings":[]},"rows":[],"variables":[],"count":0}),
                "/update" => json!({"asserted":1,"retracted":0,"tx":7}),
                _ => panic!("unexpected private route {path}"),
            }
            .to_string();
            captured
                .lock()
                .unwrap()
                .push((headers, String::from_utf8(payload).unwrap()));
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let config = root.join("home/.config/seeds/config.toml");
    let command = |actor: &str, mode: Option<&str>| {
        let mode = mode
            .map(|m| format!("actor_principal_mode={m:?}\n"))
            .unwrap_or_default();
        std::fs::write(&config,format!("[quipu]\nsigning_key_file={:?}\nsigning_session=\"host-session\"\nsigning_introducer=\"urn:test:lead\"\n{mode}",key.to_str().unwrap())).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_sd"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("home/.config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("USER", "synthetic-user")
            .current_dir(root.join("work"))
            .args([
                "--quipu",
                &url,
                "--graph",
                "urn:test:host-actors",
                "--actor",
                actor,
                "create",
                "Synthetic actor probe",
            ]);
        command
    };
    let run = |actor, mode| command(actor, mode).output().unwrap();
    let first = run("worker-a", None);
    let second = run("worker-b", None);
    let refused = run("worker-c", Some("enforce"));
    let off = run("worker-d", Some("off"));
    let matching = run("host-principal", Some("enforce"));
    // Turning off attribution must never turn off strict authentication.
    let updates_before = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(h, _)| h.starts_with("POST /update "))
        .count();
    allow_identity.store(false, Ordering::Relaxed);
    let mut strict = command("worker-g", Some("off"));
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&config)
            .unwrap(),
        "require_identity=true"
    )
    .unwrap();
    let strict_refused = strict.output().unwrap();
    let updates_after_refusal = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(h, _)| h.starts_with("POST /update "))
        .count();
    allow_identity.store(true, Ordering::Relaxed);
    let strict_allowed = strict.output().unwrap();

    // A configured key is not an effective signer for an untrusted project URL.
    std::fs::create_dir_all(root.join("work/.seeds")).unwrap();
    std::fs::write(
        root.join("work/.seeds/config.toml"),
        format!("[quipu]\nurl={:?}\n", url.replace("127.1:", "127.0.0.1:")),
    )
    .unwrap();
    let before_untrusted = requests.lock().unwrap().len();
    let untrusted = Command::new(env!("CARGO_BIN_EXE_sd"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home/.config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("USER", "synthetic-user")
        .current_dir(root.join("work"))
        .args([
            "--graph",
            "urn:test:host-actors",
            "--actor",
            "worker-h",
            "create",
            "Synthetic strict project probe",
        ])
        .output()
        .unwrap();
    let after_untrusted = requests.lock().unwrap().len();
    std::fs::remove_file(root.join("work/.seeds/config.toml")).unwrap();
    let state_path = root.join("state/seeds/actor-principal.json");
    let state: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    let held_counter = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("state/seeds/actor-principal.lock"))
        .unwrap();
    held_counter.lock().unwrap();
    let started = std::time::Instant::now();
    let mut child = command("worker-f", Some("observe"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("optional observer stalled a write while its counter lock was held");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let contended = child.wait_with_output().unwrap();
    drop(held_counter);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&state_path).unwrap()).unwrap(),
        state
    );
    std::fs::write(&state_path, b"{malformed retained counter").unwrap();
    let degraded = run("worker-e", Some("observe"));
    let before_bearer = requests.lock().unwrap().len();
    let token = root.join("synthetic.token");
    std::fs::write(&token, "synthetic-fixture-bearer").unwrap();
    std::fs::write(
        &config,
        format!("[quipu]\ntoken_file={:?}\n", token.to_str().unwrap()),
    )
    .unwrap();
    let bearer = Command::new(env!("CARGO_BIN_EXE_sd"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home/.config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("USER", "synthetic-user")
        .current_dir(root.join("work"))
        .args([
            "--quipu",
            &url,
            "--graph",
            "urn:test:host-actors",
            "show",
            "missing",
        ])
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    for output in [&first, &second, &off, &matching, &degraded, &contended] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        !strict_refused.status.success(),
        "off bypassed strict identity"
    );
    assert!(String::from_utf8_lossy(&strict_refused.stderr).contains("identity response"));
    assert_eq!(
        updates_before, updates_after_refusal,
        "strict refusal wrote an update"
    );
    assert!(
        strict_allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&strict_allowed.stderr)
    );
    assert!(
        !untrusted.status.success(),
        "untrusted project wrote without an effective signer"
    );
    assert!(
        String::from_utf8_lossy(&untrusted.stderr).contains("enrolled remote session key"),
        "{}",
        String::from_utf8_lossy(&untrusted.stderr)
    );
    assert_eq!(
        before_untrusted, after_untrusted,
        "untrusted strict write reached the server"
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr)
        .contains("differs from the authenticated principal"));
    assert!(String::from_utf8_lossy(&first.stderr).contains("mismatches_total=1"));
    assert!(String::from_utf8_lossy(&second.stderr).contains("mismatches_total=2"));
    assert!(String::from_utf8_lossy(&degraded.stderr).contains("counter UNKNOWN"));
    assert!(String::from_utf8_lossy(&contended.stderr).contains("counter UNKNOWN"));
    assert!(String::from_utf8_lossy(&contended.stderr).contains("busy after one second"));
    assert!(!bearer.status.success());
    assert!(String::from_utf8_lossy(&bearer.stderr).contains("refusing to send a bearer token"));
    assert_eq!(
        requests.lock().unwrap().len(),
        before_bearer,
        "a refused bearer must not even reach /health"
    );
    assert_eq!(
        std::fs::read(&state_path).unwrap(),
        b"{malformed retained counter"
    );
    assert_eq!(state["mismatches_total"], 2);
    assert_eq!(state["events"][0]["actor"], "worker-a");
    assert_eq!(state["events"][1]["actor"], "worker-b");
    for event in state["events"].as_array().unwrap() {
        assert_eq!(event["principal"], "host-principal");
        assert_eq!(event["verb"], "create");
        assert!(event["at"].as_str().unwrap().ends_with('Z'));
    }
    let requests = requests.lock().unwrap();
    let updates: Vec<_> = requests
        .iter()
        .filter(|(h, _)| h.starts_with("POST /update "))
        .collect();
    assert_eq!(updates.len(), 7, "enforce mismatch must never write");
    for actor in [
        "worker-a",
        "worker-b",
        "worker-d",
        "host-principal",
        "worker-e",
        "worker-f",
        "worker-g",
    ] {
        assert!(
            updates
                .iter()
                .any(|(_, body)| body.contains(&format!("&actor={actor}&"))),
            "actor {actor} replaced"
        );
    }
    for (headers, _) in updates {
        let encoded = headers
            .lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("x-quipu-attestation"))
                    .map(|(_, v)| v.trim())
            })
            .unwrap();
        let envelope: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            envelope["key_id"],
            signer.key_id(),
            "all actors use the SAME host key"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
