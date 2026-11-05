//! New provenance is optional for historical items, but typed and single-valued.
#![cfg(all(feature = "shacl", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used)]
use seeds::backend::Ctx;
use seeds::engine::{self, CreateReq};
use seeds::model::Obj;
use seeds::quipu_backend::QuipuBackend;
use seeds::validate::push_ntriples;
use seeds::vocab;

#[test]
fn both_profiles_admit_legacy_and_typed_provenance_but_refuse_bad_values() {
    let ctx = Ctx {
        now: "2026-10-22T00:00:00Z".into(),
        actor: "tester".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    };
    let mut backend = QuipuBackend::in_memory("https://example.invalid/shape-test").unwrap();
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
    let at = (
        vocab::seeds("deferredAt"),
        Obj::Typed {
            lexical: ctx.now.clone(),
            datatype: vocab::XSD_DATE_TIME.into(),
        },
    );
    let by = (
        vocab::seeds("deferredBy"),
        Obj::Iri(vocab::principal_iri(&ctx.actor)),
    );
    for shape in [
        include_str!("../shapes/seeds.shapes.ttl"),
        include_str!("../shapes/quipu-gate.shapes.ttl"),
    ] {
        let conforms = |extra: Vec<seeds::model::Fact>| {
            let mut facts = seed.facts();
            facts.extend(extra);
            let mut nt = String::new();
            push_ntriples(&mut nt, "https://example.invalid/item/control", &facts);
            quipu::validate_shapes(shape, &nt).unwrap().conforms
        };
        assert!(conforms(vec![]), "historical records require no backfill");
        assert!(conforms(vec![at.clone(), by.clone()]));
        assert!(!conforms(vec![(at.0.clone(), Obj::Str(ctx.now.clone()))]));
        assert!(!conforms(vec![(by.0.clone(), Obj::Str(ctx.actor.clone()))]));
        assert!(!conforms(vec![
            at.clone(),
            (
                at.0.clone(),
                Obj::Typed {
                    lexical: "2026-10-23T00:00:00Z".into(),
                    datatype: vocab::XSD_DATE_TIME.into()
                }
            )
        ]));
        assert!(!conforms(vec![
            by.clone(),
            (by.0.clone(), Obj::Iri(vocab::principal_iri("other")))
        ]));
    }
}
