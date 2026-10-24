//! ASK visibility is enforced before transaction commit, across engine/CLI paths.
#![allow(clippy::unwrap_used)]
use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, UpdateReq};
use seeds::error::ErrorKind;
use seeds::quipu_backend::QuipuBackend;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

fn context() -> Ctx {
    Ctx {
        now: "2026-10-22T00:00:00Z".into(),
        actor: "tester".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}
fn make(b: &mut QuipuBackend, title: &str, labels: Vec<String>) -> String {
    engine::create(
        b,
        &context(),
        &CreateReq {
            title: title.into(),
            labels,
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id
}
fn cases() -> Vec<(&'static str, Vec<String>)> {
    let mut out: Vec<_> = [
        "needs-stiwi",
        "blocked:human",
        "desk",
        "decision-needed",
        "decision-stiwi",
    ]
    .into_iter()
    .map(|label| ("Technical control", vec![label.into()]))
    .collect();
    out.push(("ASK Stiwi no colon required", vec![]));
    out
}
#[test]
fn every_ask_class_refuses_defer_and_both_update_routes_without_any_write() {
    for (title, labels) in cases() {
        let mut b = QuipuBackend::in_memory("https://example.invalid/ask-test").unwrap();
        let id = make(&mut b, title, labels);
        let before = b.snapshot(None).unwrap();
        let error = engine::defer(&mut b, &context(), std::slice::from_ref(&id), None).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Refused);
        assert!(error.message.contains("bsf8") && error.message.contains("Brief's Later"));
        for request in [
            UpdateReq {
                status: Some("deferred".into()),
                ..Default::default()
            },
            UpdateReq {
                defer: Some("2027-01-01".into()),
                ..Default::default()
            },
            UpdateReq {
                status: Some("deferred".into()),
                title: Some("disguised".into()),
                set_labels: Some(vec![]),
                ..Default::default()
            },
        ] {
            assert_eq!(
                engine::update(&mut b, &context(), std::slice::from_ref(&id), &request)
                    .unwrap_err()
                    .kind,
                ErrorKind::Refused
            );
        }
        let after = b.snapshot(None).unwrap();
        assert_eq!(after.tx, before.tx);
        assert_eq!(after.seeds, before.seeds);
    }
}
#[test]
fn mixed_batch_ask_refusal_is_atomic_in_either_order() {
    for reverse in [false, true] {
        let mut b = QuipuBackend::in_memory("https://example.invalid/ask-test").unwrap();
        let normal = make(&mut b, "Technical control", vec![]);
        let ask = make(&mut b, "ASK Stiwi: choose", vec![]);
        let before = b.snapshot(None).unwrap();
        let ids = if reverse {
            vec![ask, normal]
        } else {
            vec![normal, ask]
        };
        assert!(engine::defer_with(
            &mut b,
            &context(),
            &ids,
            Some("2027-01-01"),
            Some("must not land")
        )
        .is_err());
        assert!(engine::update(
            &mut b,
            &context(),
            &ids,
            &UpdateReq {
                status: Some("deferred".into()),
                ..Default::default()
            }
        )
        .is_err());
        let after = b.snapshot(None).unwrap();
        assert_eq!(after.tx, before.tx);
        assert_eq!(after.seeds, before.seeds);
        assert!(after.comments.is_empty());
    }
}
#[test]
fn new_ask_classification_and_deferred_creates_refuse_but_technical_defers_work() {
    for (title, labels) in cases() {
        let mut b = QuipuBackend::in_memory("https://example.invalid/ask-test").unwrap();
        let normal = make(
            &mut b,
            "Discuss ASK Stiwi syntax",
            vec!["desk-notes".into()],
        );
        let before = b.snapshot(None).unwrap();
        let req = UpdateReq {
            title: Some(title.into()),
            add_labels: labels.clone(),
            status: Some("deferred".into()),
            ..Default::default()
        };
        assert!(engine::update(&mut b, &context(), std::slice::from_ref(&normal), &req).is_err());
        for (status, defer) in [
            (Some("deferred".into()), None),
            (None, Some("2027-01-01".into())),
        ] {
            assert!(engine::create(
                &mut b,
                &context(),
                &CreateReq {
                    title: title.into(),
                    labels: labels.clone(),
                    status,
                    defer,
                    ..Default::default()
                }
            )
            .is_err());
        }
        assert_eq!(b.snapshot(None).unwrap().tx, before.tx);
        engine::defer(&mut b, &context(), std::slice::from_ref(&normal), None).unwrap();
        engine::undefer(&mut b, &context(), std::slice::from_ref(&normal)).unwrap();
        engine::update(
            &mut b,
            &context(),
            std::slice::from_ref(&normal),
            &UpdateReq {
                defer: Some("2027-01-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
}

#[test]
fn historical_hidden_ask_allows_notes_without_backfill_but_refuses_redefer() {
    use seeds::backend::{SeedWrite, WriteBatch};
    for (title, labels) in cases() {
        let mut b = QuipuBackend::in_memory("https://example.invalid/ask-test").unwrap();
        let id = make(&mut b, title, labels);
        let mut historical = b.snapshot(None).unwrap().get(&id).unwrap().clone();
        let revision = historical.revision;
        historical.status = "deferred".into();
        historical.revision += 1;
        // Reconstruct a pre-policy record in this private in-memory fixture.
        b.commit(
            &WriteBatch {
                seeds: vec![SeedWrite {
                    seed: historical,
                    expected_revision: Some(revision),
                }],
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
        engine::update(
            &mut b,
            &context(),
            std::slice::from_ref(&id),
            &UpdateReq {
                notes: Some("Retained historical finding".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let snapshot = b.snapshot(None).unwrap();
        let after = snapshot.get(&id).unwrap();
        assert_eq!(after.status, "deferred");
        assert!(!after
            .extra
            .iter()
            .any(|(p, _)| p == &seeds::vocab::seeds("deferredAt")
                || p == &seeds::vocab::seeds("deferredBy")));
        assert!(engine::defer(&mut b, &context(), std::slice::from_ref(&id), None).is_err());
        assert!(engine::update(
            &mut b,
            &context(),
            std::slice::from_ref(&id),
            &UpdateReq {
                status: Some("deferred".into()),
                ..Default::default()
            }
        )
        .is_err());
        assert_eq!(b.snapshot(None).unwrap().tx, snapshot.tx);
        engine::undefer(&mut b, &context(), std::slice::from_ref(&id)).unwrap();
    }
}

#[test]
fn labels_cannot_introduce_hidden_asks_and_batches_remain_atomic() {
    for label in [
        "decision-stiwi",
        "desk",
        "needs-stiwi",
        "blocked:human",
        "decision-needed",
    ] {
        for reverse in [false, true] {
            let mut b = QuipuBackend::in_memory("https://example.invalid/ask-test").unwrap();
            let open = make(&mut b, "Open technical", vec![]);
            let deferred = make(&mut b, "Deferred technical", vec!["technical".into()]);
            engine::defer(&mut b, &context(), std::slice::from_ref(&deferred), None).unwrap();
            let ids = if reverse {
                vec![deferred.clone(), open.clone()]
            } else {
                vec![open.clone(), deferred.clone()]
            };
            let before = b.snapshot(None).unwrap();
            assert!(engine::label_change(&mut b, &context(), &ids, label, true).is_err());
            assert_eq!(b.snapshot(None).unwrap().tx, before.tx);
            assert_eq!(b.snapshot(None).unwrap().seeds, before.seeds);
            assert!(engine::label_rename(&mut b, &context(), "technical", label).is_err());
            assert_eq!(b.snapshot(None).unwrap().tx, before.tx);
            engine::label_change(
                &mut b,
                &context(),
                std::slice::from_ref(&deferred),
                "other-technical",
                true,
            )
            .unwrap();
            engine::update(
                &mut b,
                &context(),
                std::slice::from_ref(&deferred),
                &UpdateReq {
                    status: Some("open".into()),
                    add_labels: vec![label.into()],
                    ..Default::default()
                },
            )
            .unwrap();
            engine::label_change(&mut b, &context(), std::slice::from_ref(&open), label, true)
                .unwrap();
        }
    }
}
