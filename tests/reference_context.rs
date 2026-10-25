//! Consumer validation uses only the configured project's referenced types.
#![cfg(feature = "shacl")]
#![allow(clippy::unwrap_used)]

use quipu::store::Datum;
use quipu::types::{Op, Value};
use seeds::backend::{Backend, Ctx, SeedWrite, WriteBatch};
use seeds::engine::{self, CreateReq};
use seeds::model::Obj;
use seeds::quipu_backend::QuipuBackend;
use seeds::vocab;

const GRAPH: &str = "urn:test:reference-project";
const NOW: &str = "2026-10-25T00:00:00Z";
const LINK: &str = "urn:test:reference";
const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
<urn:test:ReferenceShape> a sh:NodeShape ;
 sh:targetClass <https://schema.org/Action> ;
 sh:property [ sh:path <urn:test:reference> ;
 sh:class <https://schema.org/Action> ; sh:maxCount 1 ] .";

fn context() -> Ctx {
    Ctx {
        now: NOW.into(),
        actor: "fixture".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}

fn create(b: &mut QuipuBackend, title: &str, ephemeral: bool) -> String {
    engine::create(
        b,
        &context(),
        &CreateReq {
            title: title.into(),
            ephemeral,
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id
}

fn datum(b: &QuipuBackend, subject: &str, property: &str, target: &str, op: Op) -> Datum {
    Datum {
        entity: b.store().intern(subject).unwrap(),
        attribute: b.store().intern(property).unwrap(),
        value: Value::Ref(b.store().intern(target).unwrap()),
        valid_from: NOW.into(),
        valid_to: None,
        op,
    }
}

fn write(b: &mut QuipuBackend, graph: i64, datums: Vec<Datum>) {
    b.store_mut()
        .transact_graph_batches(&[(graph, datums)], NOW, Some("fixture"), Some("fixture"))
        .unwrap();
}

fn check_target_scope(place: &str) {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    // Match the native CLI: an in-memory core backend defaults to no store gate.
    b.store_mut().shacl_config_mut().validate_on_write = true;
    assert!(b.store().shacl_config().validate_on_write);
    let target = create(&mut b, "target", false);
    let source = create(&mut b, "source", true);
    let target_iri = vocab::item_iri(&target);
    let g = b.store().graph_create(GRAPH).unwrap();
    if place == "pseudo" {
        let changes = vec![
            datum(
                &b,
                &target_iri,
                vocab::RDF_TYPE,
                &vocab::schema("Action"),
                Op::Retract,
            ),
            datum(
                &b,
                &target_iri,
                vocab::RDF_TYPE,
                &vocab::schema("PseudoAction"),
                Op::Assert,
            ),
        ];
        write(&mut b, g, changes);
    } else if place != "project" {
        let other = if place == "root" {
            quipu::schema::ROOT_GRAPH
        } else {
            b.store()
                .graph_create("urn:test:unrelated-project")
                .unwrap()
        };
        let entity = b.store().lookup(&target_iri).unwrap().unwrap();
        let facts = b.store().entity_facts_in_graph(entity, g).unwrap();
        let changes = |op| {
            facts
                .iter()
                .map(|f| Datum {
                    entity,
                    attribute: f.attribute,
                    value: f.value.clone(),
                    valid_from: f.valid_from.clone(),
                    valid_to: None,
                    op,
                })
                .collect()
        };
        b.store_mut()
            .transact_graph_batches(
                &[(g, changes(Op::Retract)), (other, changes(Op::Assert))],
                NOW,
                Some("fixture"),
                Some("fixture"),
            )
            .unwrap();
    }
    // The initial source has no reference and conforms to this optional rule.
    // The checked batch introduces the first value: no legacy-debt exemption.
    b.store_mut()
        .load_shapes("reference-fixture", SHAPES, NOW)
        .unwrap();
    let before = b.store().transaction_head().unwrap();
    let snapshot = b.snapshot_items(std::slice::from_ref(&source)).unwrap();
    let mut seed = snapshot.seeds.get(&source).unwrap().clone();
    let expected_revision = seed.revision;
    seed.revision += 1;
    seed.title = "updated".into();
    seed.extra.retain(|(property, _)| property != LINK);
    seed.extra.insert((LINK.into(), Obj::Iri(target_iri)));
    let result = b.commit(
        &WriteBatch {
            seeds: vec![SeedWrite {
                seed,
                expected_revision: Some(expected_revision),
            }],
            source: "fixture".into(),
            ..Default::default()
        },
        &context(),
    );
    if place == "project" {
        assert!(result.is_ok(), "{result:?}");
        assert!(b.store().transaction_head().unwrap() > before);
    } else {
        assert!(
            result.is_err(),
            "{place} supplied an unauthorized Action type"
        );
        assert_eq!(
            b.store().transaction_head().unwrap(),
            before,
            "refusal committed facts"
        );
    }
}

#[test]
fn ephemeral_write_accepts_its_configured_project_type() {
    check_target_scope("project");
}
#[test]
fn unrelated_project_type_cannot_validate_an_ephemeral_reference() {
    check_target_scope("other");
}
#[test]
fn root_type_cannot_validate_an_ephemeral_reference() {
    check_target_scope("root");
}
#[test]
fn pseudo_action_is_not_an_action() {
    check_target_scope("pseudo");
}
