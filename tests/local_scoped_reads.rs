//! aegis-xdvg.4: a local quipu store answers current-state reads from scoped
//! reads (only the seeds a command answers from) and prints exactly what a
//! whole snapshot prints. A read pinned at the head takes the whole-snapshot
//! path, so the two are compared on one store.
#![allow(clippy::unwrap_used)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, ListReq, ReadyReq};
use seeds::quipu_backend::QuipuBackend;

const GRAPH: &str = "https://seeds.local/project/sd";

fn ctx(n: u32) -> Ctx {
    Ctx {
        now: format!("2026-09-30T00:{:02}:{:02}Z", (n / 60) % 60, n % 60),
        actor: "tester".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}

fn mk(b: &mut QuipuBackend, title: &str, n: u32) -> String {
    let req = CreateReq {
        title: title.into(),
        ..CreateReq::default()
    };
    engine::create(b, &ctx(n), &req).unwrap().0.id
}

/// open, blocked by an open seed, blocked by a closed seed, deferred (past and
/// future), a parent/child pair, a related edge and comments.
fn board() -> (QuipuBackend, Vec<String>) {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let ids: Vec<String> = (0..9)
        .map(|i| mk(&mut b, &format!("seed {i}"), i))
        .collect();
    engine::dep_add(&mut b, &ctx(20), &ids[1], &ids[2], "blocks").unwrap();
    engine::dep_add(&mut b, &ctx(21), &ids[3], &ids[4], "blocks").unwrap();
    engine::close(&mut b, &ctx(22), &[ids[4].clone()], Some("done"), false).unwrap();
    engine::defer(
        &mut b,
        &ctx(23),
        &[ids[5].clone()],
        Some("2020-01-01T00:00:00Z"),
    )
    .unwrap();
    engine::defer(
        &mut b,
        &ctx(24),
        &[ids[6].clone()],
        Some("2099-01-01T00:00:00Z"),
    )
    .unwrap();
    engine::dep_add(&mut b, &ctx(25), &ids[7], &ids[0], "parent-child").unwrap();
    engine::dep_add(&mut b, &ctx(26), &ids[8], &ids[0], "related").unwrap();
    engine::comment_add(&mut b, &ctx(27), &ids[0], "first", None).unwrap();
    engine::comment_add(&mut b, &ctx(28), &ids[2], "second", None).unwrap();
    (b, ids)
}

fn head(b: &QuipuBackend) -> Option<u64> {
    Some(u64::try_from(b.store().transaction_head().unwrap()).unwrap())
}

#[test]
fn local_store_takes_scoped_reads() {
    let (b, _) = board();
    assert!(b.scoped_reads());
}

#[test]
fn scoped_ready_prints_what_a_whole_snapshot_prints() {
    let (b, ids) = board();
    let req = ReadyReq::default();
    let scoped = engine::ready(&b, &ctx(40), &req, None).unwrap();
    let whole = engine::ready(&b, &ctx(40), &req, head(&b)).unwrap();
    assert_eq!(
        format!("{:?}", scoped.issues),
        format!("{:?}", whole.issues)
    );
    let got: Vec<&str> = scoped.issues.iter().map(|s| s.id.as_str()).collect();
    // blocked by an open seed and deferred to the future are not ready.
    assert!(
        !got.contains(&ids[1].as_str()) && !got.contains(&ids[6].as_str()),
        "{got:?}"
    );
    // blocked only by a closed seed is ready.
    assert!(got.contains(&ids[3].as_str()), "{got:?}");
}

#[test]
fn scoped_ready_ids_match_the_correlated_query() {
    let (b, _) = board();
    assert_eq!(b.ready_ids(None).unwrap(), b.ready_ids(head(&b)).unwrap());
}

#[test]
fn scoped_list_prints_what_a_whole_snapshot_prints() {
    let (b, _) = board();
    for all in [false, true] {
        let req = ListReq {
            all,
            ..ListReq::default()
        };
        let scoped = engine::list(&b, &req, None).unwrap();
        let whole = engine::list(&b, &req, head(&b)).unwrap();
        assert_eq!(
            format!("{:?}", scoped.issues),
            format!("{:?}", whole.issues)
        );
        assert_eq!(scoped.dependent_counts, whole.dependent_counts);
    }
}

#[test]
fn scoped_show_prints_what_a_whole_snapshot_prints() {
    let (b, ids) = board();
    for id in &ids {
        let scoped = engine::show(&b, std::slice::from_ref(id), None).unwrap();
        let whole = engine::show(&b, std::slice::from_ref(id), head(&b)).unwrap();
        assert_eq!(format!("{scoped:?}"), format!("{whole:?}"), "{id}");
    }
}
