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
    for route in 0..4 {
        let mut b = QuipuBackend::in_memory("https://example.invalid/provenance").unwrap();
        let ctx = context();
        let seed = engine::create(
            &mut b,
            &ctx,
            &CreateReq {
                title: "Technical control".into(),
                status: (route == 0).then(|| "deferred".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        let id = seed.id;
        match route {
            0 => (),
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
        let next = Ctx {
            now: "2026-10-23T00:00:00Z".into(),
            actor: "other".into(),
            ..context()
        };
        engine::defer(&mut b, &next, std::slice::from_ref(&id), None).unwrap();
        stamp(b.snapshot(None).unwrap().get(&id).unwrap(), &next);
        engine::undefer(&mut b, &next, std::slice::from_ref(&id)).unwrap();
        stamp(b.snapshot(None).unwrap().get(&id).unwrap(), &next);
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
