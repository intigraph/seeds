//! All SDK/CLI producers use the same atomic engine deferral stamps.
#![allow(clippy::unwrap_used)]
use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, UpdateReq};
use seeds::model::{Obj, Seed};
use seeds::quipu_backend::QuipuBackend;
use seeds::vocab;
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
fn stamp(seed: &Seed, ctx: &Ctx) {
    assert!(seed.extra.contains(&(
        vocab::seeds("deferredAt"),
        Obj::Typed {
            lexical: ctx.now.clone(),
            datatype: vocab::XSD_DATE_TIME.into()
        }
    )));
    assert!(seed.extra.contains(&(
        vocab::seeds("deferredBy"),
        Obj::Iri(vocab::principal_iri(&ctx.actor))
    )));
    for p in ["deferredAt", "deferredBy"] {
        assert_eq!(
            seed.extra
                .iter()
                .filter(|(predicate, _)| predicate == &vocab::seeds(p))
                .count(),
            1
        );
    }
}
#[test]
fn every_explicit_deferral_route_stamps_and_redefer_replaces_only_latest_pair() {
    for route in 0..5 {
        let mut b = QuipuBackend::in_memory("https://example.invalid/provenance").unwrap();
        let ctx = context();
        let seed = engine::create(
            &mut b,
            &ctx,
            &CreateReq {
                title: "Technical control".into(),
                status: (route == 0).then(|| "deferred".into()),
                defer: (route == 4).then(|| "2026-10-23T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        let id = seed.id;
        match route {
            0 | 4 => (),
            1 => {
                engine::defer(&mut b, &ctx, std::slice::from_ref(&id), None).unwrap();
            }
            2 => {
                engine::update(
                    &mut b,
                    &ctx,
                    std::slice::from_ref(&id),
                    &UpdateReq {
                        status: Some("deferred".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            _ => {
                engine::update(
                    &mut b,
                    &ctx,
                    std::slice::from_ref(&id),
                    &UpdateReq {
                        defer: Some("2026-10-23T00:00:00Z".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        }
        stamp(b.snapshot(None).unwrap().get(&id).unwrap(), &ctx);
        assert!(b
            .snapshot(None)
            .unwrap()
            .get(&id)
            .unwrap()
            .extra
            .contains(&(vocab::seeds("observedDeferralWriteCount"), Obj::Int(1))));
        // An identical generic update must not manufacture a counter write.
        if route != 3 {
            let before = b.snapshot(None).unwrap();
            engine::update(
                &mut b,
                &ctx,
                std::slice::from_ref(&id),
                &UpdateReq {
                    status: Some("deferred".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            let after = b.snapshot(None).unwrap();
            assert_eq!(before.seeds, after.seeds);
        }

        let next = Ctx {
            now: "2026-10-23T00:00:00Z".into(),
            actor: "other".into(),
            ..context()
        };
        engine::defer(&mut b, &next, std::slice::from_ref(&id), None).unwrap();
        stamp(b.snapshot(None).unwrap().get(&id).unwrap(), &next);
        let snapshot = b.snapshot(None).unwrap();
        let current = snapshot.get(&id).unwrap();
        assert!(current
            .extra
            .contains(&(vocab::seeds("observedDeferralWriteCount"), Obj::Int(2))));
        assert!(current.extra.contains(&(
            vocab::seeds("observedDeferralWriteCountSince"),
            Obj::Typed {
                lexical: ctx.now.clone(),
                datatype: vocab::XSD_DATE_TIME.into()
            }
        )));

        engine::undefer(&mut b, &next, std::slice::from_ref(&id)).unwrap();
        stamp(b.snapshot(None).unwrap().get(&id).unwrap(), &next);
        let snapshot = b.snapshot(None).unwrap();
        let current = snapshot.get(&id).unwrap();
        assert!(current
            .extra
            .contains(&(vocab::seeds("observedDeferralWriteCount"), Obj::Int(2))));
        assert!(current.extra.contains(&(
            vocab::seeds("observedDeferralWriteCountSince"),
            Obj::Typed {
                lexical: ctx.now.clone(),
                datatype: vocab::XSD_DATE_TIME.into()
            }
        )));
    }
}
#[test]
fn no_backfill_on_unrelated_edit_and_invalid_context_cannot_partially_commit() {
    let mut b = QuipuBackend::in_memory("https://example.invalid/provenance").unwrap();
    let ctx = context();
    let id = engine::create(
        &mut b,
        &ctx,
        &CreateReq {
            title: "Legacy control".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id;
    engine::update(
        &mut b,
        &ctx,
        std::slice::from_ref(&id),
        &UpdateReq {
            notes: Some("unrelated edit".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!b
        .snapshot(None)
        .unwrap()
        .get(&id)
        .unwrap()
        .extra
        .iter()
        .any(|(p, _)| p == &vocab::seeds("deferredAt") || p == &vocab::seeds("deferredBy")));
    for bad in [
        Ctx {
            actor: " ".into(),
            ..context()
        },
        Ctx {
            now: "invalid".into(),
            ..context()
        },
    ] {
        let before = b.snapshot(None).unwrap();
        assert!(engine::defer_with(
            &mut b,
            &bad,
            std::slice::from_ref(&id),
            None,
            Some("must not commit")
        )
        .is_err());
        let after = b.snapshot(None).unwrap();
        assert_eq!(before.tx, after.tx);
        assert_eq!(before.seeds, after.seeds);
        assert_eq!(before.comments, after.comments);
    }
}

#[test]
fn overflow_refuses_whole_multi_item_deferral_without_comment_or_revision() {
    use seeds::backend::{SeedWrite, WriteBatch};
    let mut backend = QuipuBackend::in_memory("https://example.invalid/count-overflow").unwrap();
    let ctx = context();
    let first = engine::create(
        &mut backend,
        &ctx,
        &CreateReq {
            title: "First".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let mut full = engine::create(
        &mut backend,
        &ctx,
        &CreateReq {
            title: "Counter at limit".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let revision = full.revision;
    full.revision += 1;
    full.extra.insert((
        vocab::seeds("observedDeferralWriteCount"),
        Obj::Int(i64::MAX),
    ));
    full.extra.insert((
        vocab::seeds("observedDeferralWriteCountSince"),
        Obj::Typed {
            lexical: ctx.now.clone(),
            datatype: vocab::XSD_DATE_TIME.into(),
        },
    ));
    let mut batch = WriteBatch::default();
    batch.seeds.push(SeedWrite {
        seed: full.clone(),
        expected_revision: Some(revision),
    });
    backend.commit(&batch, &ctx).unwrap();
    let before = backend.snapshot(None).unwrap();
    assert!(engine::defer_with(
        &mut backend,
        &ctx,
        &[first.id, full.id],
        None,
        Some("must not land")
    )
    .is_err());
    let after = backend.snapshot(None).unwrap();
    assert_eq!(before.tx, after.tx);
    assert_eq!(before.seeds, after.seeds);
    assert_eq!(before.comments, after.comments);
}

#[test]
fn malformed_pair_refuses_backend_batch_and_preserves_unknown_facts() {
    use seeds::backend::{SeedWrite, WriteBatch};
    let mut backend = QuipuBackend::in_memory("https://example.invalid/count-validation").unwrap();
    let ctx = context();
    let seed = engine::create(
        &mut backend,
        &ctx,
        &CreateReq {
            title: "Technical control".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    for count in [Obj::Int(0), Obj::Int(-1), Obj::Str("1".into()), Obj::Int(1)] {
        let mut invalid = seed.clone();
        invalid.revision += 1;
        invalid
            .extra
            .insert((vocab::seeds("observedDeferralWriteCount"), count));
        let mut batch = WriteBatch::default();
        batch.seeds.push(SeedWrite {
            seed: invalid,
            expected_revision: Some(seed.revision),
        });
        let before = backend.snapshot(None).unwrap();
        assert!(backend.commit(&batch, &ctx).is_err());
        let after = backend.snapshot(None).unwrap();
        assert_eq!(before.tx, after.tx);
        assert_eq!(before.seeds, after.seeds);
    }
    let unknown = (
        "https://example.invalid/unknown".to_string(),
        Obj::Str("preserve".into()),
    );
    let mut carried = seed.clone();
    carried.revision += 1;
    carried.extra.insert(unknown.clone());
    let mut batch = WriteBatch::default();
    batch.seeds.push(SeedWrite {
        seed: carried,
        expected_revision: Some(seed.revision),
    });
    backend.commit(&batch, &ctx).unwrap();
    engine::defer(&mut backend, &ctx, std::slice::from_ref(&seed.id), None).unwrap();
    engine::update(
        &mut backend,
        &ctx,
        std::slice::from_ref(&seed.id),
        &UpdateReq {
            notes: Some("ordinary edit".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let snapshot = backend.snapshot(None).unwrap();
    let current = snapshot.get(&seed.id).unwrap();
    assert!(current.extra.contains(&unknown));
    assert!(current
        .extra
        .contains(&(vocab::seeds("observedDeferralWriteCount"), Obj::Int(1))));
}
