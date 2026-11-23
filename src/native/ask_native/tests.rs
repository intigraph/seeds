use super::*;
#[test]
fn caller_cannot_supply_a_verified_answer() {
    let forged = json!({"graph":"urn:project","ask":"a-1","ask_revision":1,"target":"a-2","target_revision":2,
        "receipt":format!("urn:quipu:attest:{}","0".repeat(64)),"verified":true});
    assert!(serde_json::from_value::<Binding>(forged).is_err());
}

#[test]
fn default_cli_does_not_initialize_a_store() {
    use clap::Parser;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("absent.db");
    let cli =
        Cli::try_parse_from(["sd", "--store", store.to_str().unwrap(), "ask-settle"]).unwrap();
    let outcome = crate::native::run(&cli);
    assert_ne!(outcome.code, 0);
    assert!(!store.exists());
}

#[test]
fn reservation_is_exclusive_fsynced_and_survives_code_changes() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let uid = std::fs::metadata(dir.path()).unwrap().uid();
    let identity = json!({"store":"fixture", "receipt":"one-intent"});
    custody::reserve(
        dir.path(),
        uid,
        &identity,
        &json!({"code":"first", "state":"SEND_RESERVED"}),
    )
    .unwrap();
    let files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|f| f.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let before = std::fs::read(&files[0]).unwrap();
    assert!(custody::reserve(dir.path(), uid, &identity, &json!({"code":"replacement"})).is_err());
    assert_eq!(std::fs::read(&files[0]).unwrap(), before);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(custody::reserve(dir.path(), uid, &json!({"receipt":"other"}), &json!({})).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn private_credential_refuses_broad_mode_and_symlink() {
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("credential");
    std::fs::write(&key, "synthetic-fixture-only").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let uid = std::fs::metadata(&key).unwrap().uid();
    assert!(custody::private_file(&key, uid).is_ok());
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(custody::private_file(&key, uid).is_err());
    let link = dir.path().join("link");
    symlink(&key, &link).unwrap();
    assert!(custody::private_file(&link, uid).is_err());
    assert!(profile::checked_file(&key, 0, true).is_err());
}

/// # arming: agent-invoked with the explicit disposable native/TLS fixture.
/// This bypasses ONLY root deployment selection inside a test binary. The
/// release command has no such override; it always checks root custody/code.
#[test]
#[ignore = "requires the separately qualified native receipt/TLS fixture"]
fn actual_native_process_receipt_drives_one_atomic_settlement() {
    use crate::backend::{Backend, SeedWrite, WriteBatch};
    use crate::engine::{self, CreateReq};
    use crate::quipu_backend::QuipuBackend;
    let path = std::env::var("SEEDS_NATIVE_PORT_FIXTURE").expect("explicit fixture required");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let profile: profile::Profile = serde_json::from_value(fixture["profile"].clone()).unwrap();
    let request: Request = serde_json::from_value::<Binding>(fixture["binding"].clone())
        .unwrap()
        .request();
    assert_eq!(profile.graph, "https://example.test/items");
    assert_eq!(profile.machine_principal, "fixture-settler");
    let url = url::Url::parse(&profile.endpoint).unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("localhost"));
    let ca = fixture["ca"].as_str().unwrap().to_string();
    let ctx = Ctx {
        now: super::super::now(),
        actor: "fixture-settler".into(),
        prefix: "fixture".into(),
        claims: Default::default(),
    };
    let mut templates = QuipuBackend::in_memory("urn:fixture:templates").unwrap();
    let mut target = engine::create(
        &mut templates,
        &ctx,
        &CreateReq {
            title: "Native verified choice".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let mut ask = engine::create(
        &mut templates,
        &ctx,
        &CreateReq {
            title: "Choose".into(),
            issue_type: Some("ask".into()),
            unblocks: Some(target.id.clone()),
            default: Some("No".into()),
            expires_at: Some("2030-01-01T00:00:00Z".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    target.id = request.target.clone();
    target.revision = request.target_revision;
    ask.id = request.ask.clone();
    ask.revision = request.ask_revision;
    ask.unblocks = Some(target.id.clone());
    let mut backend = QuipuBackend::in_memory(&request.graph).unwrap();
    backend
        .commit(
            &WriteBatch {
                seeds: vec![
                    SeedWrite {
                        seed: target,
                        expected_revision: None,
                    },
                    SeedWrite {
                        seed: ask,
                        expected_revision: None,
                    },
                ],
                source: "fixture:create".into(),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    let port = verifier::NativePort::new(profile.clone(), request.clone(), Some(ca.clone()));
    let initial = backend.snapshot(None).unwrap().tx;
    let mut wrong = request.clone();
    wrong.target_revision += 1;
    let wrong_port = verifier::NativePort::new(profile.clone(), wrong.clone(), Some(ca.clone()));
    assert!(ask_settlement::settle(&mut backend, &ctx, &wrong, Some(&wrong_port)).is_err());
    assert_eq!(backend.snapshot(None).unwrap().tx, initial);
    assert_eq!(
        std::fs::read_dir(&profile.custody_directory)
            .unwrap()
            .count(),
        0
    );
    let settled = ask_settlement::settle(&mut backend, &ctx, &request, Some(&port)).unwrap();
    assert!(!settled.already_settled);
    let snapshot = backend.snapshot(None).unwrap();
    assert_eq!(snapshot.get(&request.ask).unwrap().status, "closed");
    assert_eq!(snapshot.get(&request.target).unwrap().status, "open");
    assert_eq!(snapshot.comments_on(&request.target).len(), 1);
    assert!(snapshot.comments_on(&request.target)[0]
        .text
        .contains("Operator ruling: Yes"));
    assert_eq!(
        std::fs::read_dir(&profile.custody_directory)
            .unwrap()
            .count(),
        1
    );
    let tx = snapshot.tx;
    // A new process/profile instance re-verifies the actual native receipt.
    let recovered = verifier::NativePort::new(profile.clone(), request.clone(), Some(ca));
    assert!(
        ask_settlement::reconcile(&backend, &request, Some(&recovered))
            .unwrap()
            .already_settled
    );
    assert!(
        ask_settlement::settle(&mut backend, &ctx, &request, Some(&recovered))
            .unwrap()
            .already_settled
    );
    assert_eq!(backend.snapshot(None).unwrap().tx, tx);
    assert!(recovered.reserve_send(&request).is_err());
    assert_eq!(
        std::fs::read_dir(&profile.custody_directory)
            .unwrap()
            .count(),
        1
    );
}
