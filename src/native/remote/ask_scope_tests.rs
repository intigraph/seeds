//! Explicit disposable scope qualification; no production credential or store.
use super::*;
use crate::backend::{Backend, Ctx, SeedWrite, WriteBatch};
use crate::engine::{self, CreateReq};
use serde_json::json;
use std::path::Path;
use std::process::{Child, Command, Stdio};

struct Fixture {
    child: Child,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn start(bin: &str, dir: &Path) -> (Fixture, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let child = Command::new(bin)
        .args([
            "--db",
            dir.join("fixture.db").to_str().unwrap(),
            "--bind",
            &address.to_string(),
        ])
        .current_dir(dir)
        .env("HOME", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut fixture = Fixture { child };
    let base = format!("http://{address}");
    for _ in 0..100 {
        assert!(
            fixture.child.try_wait().unwrap().is_none(),
            "fixture server exited"
        );
        if ureq::get(&format!("{base}/health")).call().is_ok() {
            return (fixture, base);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("fixture server readiness timeout");
}
fn local(cli: &str, dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(cli)
        .args(args)
        .args(["--db", dir.join("fixture.db").to_str().unwrap()])
        .current_dir(dir)
        .env("HOME", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture owner CLI failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
#[ignore = "requires explicitly pinned native CLI and HTTP server artifacts"]
fn actual_scoped_writer_accepts_only_project_local_settler_audit() {
    let bin = std::env::var("SEEDS_TEST_QUIPU_SERVER").expect("explicit server");
    let cli = std::env::var("SEEDS_TEST_QUIPU_CLI").expect("explicit owner CLI");
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".bobbin")).unwrap();
    let registry = json!({"version":1,"credentials":[{"credential_id":"fixture-settler","principal":"urn:fixture:settler","audience":"quipu","token_sha256":crate::native::attest::sha256_hex(b"fixture-scoped-settler")}]});
    std::fs::write(dir.path().join("registry.json"), registry.to_string()).unwrap();
    std::fs::write(
        dir.path().join(".bobbin/config.toml"),
        format!(
            "[quipu.server]\nauth_token = 'fixture-owner'\nscoped_writer_credentials_file = '{}'\n",
            dir.path().join("registry.json").display()
        ),
    )
    .unwrap();
    let graph = "urn:fixture:scoped-settler";
    let ctx = Ctx {
        now: "2026-11-23T00:00:00Z".into(),
        actor: "fixture-settler".into(),
        prefix: "fixture".into(),
        claims: Default::default(),
    };
    let (server, base) = start(&bin, dir.path());
    let mut owner = RemoteBackend::connect(
        &base,
        graph,
        Some("fixture-owner".into()),
        None,
        &["127.0.0.1".into()],
    )
    .unwrap();
    let seed = engine::create(
        &mut owner,
        &ctx,
        &CreateReq {
            title: "Scoped fixture".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    drop(owner);
    drop(server); // Never open a second writer on the live database.
    let instance = local(&cli, dir.path(), &["graph", "writer-instance", graph]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let grant = json!({"version":1,"id":"fixture-settler","principal":"urn:fixture:settler","credential_id":"fixture-settler","instance":instance,"operations":["update"],"valid_from":0,"valid_until":now+3600,"revision":1,"revoked":false});
    let grant_path = dir.path().join("grant.json");
    std::fs::write(&grant_path, grant.to_string()).unwrap();
    local(
        &cli,
        dir.path(),
        &["graph", "writer-grant", grant_path.to_str().unwrap()],
    );
    let (server, base) = start(&bin, dir.path());
    let connect = || {
        RemoteBackend::connect(
            &base,
            graph,
            Some("fixture-scoped-settler".into()),
            None,
            &["127.0.0.1".into()],
        )
        .unwrap()
    };
    let mut next = seed.clone();
    next.revision += 1;
    let batch = WriteBatch {
        seeds: vec![SeedWrite {
            seed: next.clone(),
            expected_revision: Some(seed.revision),
        }],
        source: "seeds:ask-settlement".into(),
        ..Default::default()
    };
    let mut ordinary = connect();
    assert!(
        ordinary.commit(&batch, &ctx).is_err(),
        "sibling provenance must exceed the one-graph grant"
    );
    assert_eq!(
        ordinary
            .snapshot(None)
            .unwrap()
            .get(&seed.id)
            .unwrap()
            .revision,
        seed.revision
    );
    let mut admitted = connect().with_project_settler_audit();
    admitted.commit(&batch, &ctx).unwrap();
    assert_eq!(
        admitted
            .snapshot(None)
            .unwrap()
            .get(&seed.id)
            .unwrap()
            .revision,
        next.revision
    );
    admitted.settler_audit_for(&seed.id, next.revision).unwrap();
    let foreign = ureq::post(&format!("{base}/update"))
        .set("Authorization", "Bearer fixture-scoped-settler")
        .set("Content-Type", "application/sparql-update")
        .send_string("INSERT DATA { GRAPH <urn:fixture:foreign> { <urn:s> <urn:p> <urn:o> } }");
    assert!(
        matches!(foreign, Err(ureq::Error::Status(403, _))),
        "foreign graph result: {foreign:?}"
    );
    drop(ordinary);
    drop(admitted);
    drop(server);
    let mut revoked = grant.clone();
    revoked["revision"] = json!(2);
    revoked["revoked"] = json!(true);
    std::fs::write(&grant_path, revoked.to_string()).unwrap();
    local(
        &cli,
        dir.path(),
        &["graph", "writer-grant", grant_path.to_str().unwrap()],
    );
    let (_server, base) = start(&bin, dir.path());
    let mut revoked = RemoteBackend::connect(
        &base,
        graph,
        Some("fixture-scoped-settler".into()),
        None,
        &["127.0.0.1".into()],
    )
    .unwrap()
    .with_project_settler_audit();
    let mut third = next.clone();
    third.revision += 1;
    assert!(revoked
        .commit(
            &WriteBatch {
                seeds: vec![SeedWrite {
                    seed: third,
                    expected_revision: Some(next.revision)
                }],
                source: "seeds:ask-settlement".into(),
                ..Default::default()
            },
            &ctx
        )
        .is_err());
    assert_eq!(
        revoked
            .snapshot(None)
            .unwrap()
            .get(&seed.id)
            .unwrap()
            .revision,
        next.revision
    );
}
