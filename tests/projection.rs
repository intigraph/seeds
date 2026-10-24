//! Consumer fixtures, not a second feed applier. Existing engine writes stand
//! for a shared-applier commit; no network or production store is opened.
#![allow(clippy::unwrap_used)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use quipu::store::Store;
use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, ListReq, ReadyReq, UpdateReq};
use seeds::projection::{AppliedProgress, BoardProjection, SourceBinding};
use seeds::quipu_backend::QuipuBackend;
use seeds::vocab;

const GRAPH: &str = "urn:test:board";

fn ctx() -> Ctx {
    Ctx {
        now: "2026-10-01T00:00:00Z".into(),
        actor: "fixture".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}

fn binding() -> SourceBinding {
    SourceBinding {
        store_id: "fixture-source".into(),
        log_epoch: "epoch-one".into(),
        scope_digest: "board-and-ephemeral".into(),
        policy_digest: "single-principal".into(),
    }
}

fn progress(local_tx: u64) -> AppliedProgress {
    AppliedProgress {
        source: binding(),
        // Intentionally unrelated to the small local commit number.
        source_tx: 900_000,
        local_tx,
        graphs: [GRAPH.into(), vocab::ephemeral_graph(GRAPH)].into(),
        complete: true,
        heartbeat_age_ms: Some(0),
    }
}

fn read_only(mut writer: QuipuBackend) -> QuipuBackend {
    let store = std::mem::replace(writer.store_mut(), Store::open_in_memory().unwrap());
    QuipuBackend::read_only(store, GRAPH)
}

fn create(b: &mut QuipuBackend, title: &str) -> String {
    engine::create(
        b,
        &ctx(),
        &CreateReq {
            title: title.into(),
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id
}

fn view(b: QuipuBackend, p: AppliedProgress) -> BoardProjection {
    BoardProjection::from_applied(read_only(b), p, &binding(), 15_000).unwrap()
}

#[test]
fn board_plate_and_ready_use_the_existing_definitions_at_the_applied_commit() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let blocker = create(&mut b, "blocker");
    let dependent = create(&mut b, "dependent");
    engine::dep_add(&mut b, &ctx(), &dependent, &blocker, "blocks").unwrap();
    let before = b.snapshot(None).unwrap().tx;
    let old_ready = engine::ready(&b, &ctx(), &ReadyReq::default(), None).unwrap();
    let old_board = engine::list(&b, &ListReq::default(), None).unwrap();

    // A later local commit must not leak into a view pinned to the earlier
    // applier receipt. Using source_tx as a local tx would expose this close.
    engine::close(
        &mut b,
        &ctx(),
        std::slice::from_ref(&blocker),
        Some("fixture"),
        false,
    )
    .unwrap();
    engine::update(
        &mut b,
        &ctx(),
        std::slice::from_ref(&dependent),
        &UpdateReq {
            assignee: Some("fixture-reader".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let p = view(b, progress(before));
    let board = p.list(&ListReq::default(), Some(20)).unwrap();
    assert_eq!(board.value.issues, old_board.issues);
    let ready = p.ready(&ctx(), &ReadyReq::default(), Some(21)).unwrap();
    assert_eq!(ready.value.issues, old_ready.issues);
    assert_eq!(
        ready.value.issues.iter().map(|s| &s.id).collect::<Vec<_>>(),
        vec![&blocker]
    );
    assert_eq!(ready.progress.source_tx, 900_000);
    assert_eq!(ready.progress.local_tx, before);
    assert_eq!(ready.progress.heartbeat_age_ms, Some(21));
    assert_eq!(ready.bounded_freshness_ms, None);
    let mut plate = ListReq::default();
    plate.filter.assignee = Some("fixture-reader".into());
    assert!(p.list(&plate, Some(22)).unwrap().value.issues.is_empty());
}

#[test]
fn newly_committed_changes_become_visible_without_a_remote_query() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let blocker = create(&mut b, "blocker");
    let dependent = create(&mut b, "dependent");
    engine::dep_add(&mut b, &ctx(), &dependent, &blocker, "blocks").unwrap();
    engine::close(&mut b, &ctx(), &[blocker], Some("fixture"), false).unwrap();
    engine::update(
        &mut b,
        &ctx(),
        std::slice::from_ref(&dependent),
        &UpdateReq {
            assignee: Some("fixture-reader".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let head = b.snapshot(None).unwrap().tx;
    let p = view(b, progress(head));
    let mut plate = ListReq::default();
    plate.filter.assignee = Some("fixture-reader".into());
    for _ in 0..20 {
        assert_eq!(
            p.list(&plate, Some(10)).unwrap().value.issues[0].id,
            dependent
        );
        assert_eq!(
            p.ready(&ctx(), &ReadyReq::default(), Some(10))
                .unwrap()
                .value
                .issues[0]
                .id,
            dependent
        );
    }
    // Even a live heartbeat is explicitly not a <=5s freshness proof.
    assert_eq!(p.list(&plate, Some(0)).unwrap().bounded_freshness_ms, None);
    assert!(p.list(&plate, None).is_err());
    assert!(p.ready(&ctx(), &ReadyReq::default(), Some(15_001)).is_err());
}

#[test]
fn missing_incomplete_foreign_or_ahead_progress_never_becomes_an_empty_board() {
    for fault in 0..9 {
        let b = QuipuBackend::in_memory(GRAPH).unwrap();
        let head = b.snapshot(None).unwrap().tx;
        let mut p = progress(head);
        match fault {
            0 => p.complete = false,
            1 => p.graphs.clear(),
            2 => {
                p.graphs.remove(&vocab::ephemeral_graph(GRAPH));
            }
            3 => p.source.store_id = "foreign".into(),
            4 => p.source.log_epoch = "restored".into(),
            5 => p.source.scope_digest = "different".into(),
            6 => p.source.policy_digest = "different".into(),
            7 => p.source.policy_digest.clear(),
            _ => p.local_tx = head + 1,
        }
        assert!(
            BoardProjection::from_applied(read_only(b), p, &binding(), 15_000).is_err(),
            "fault {fault}"
        );
    }
    let b = QuipuBackend::in_memory(GRAPH).unwrap();
    let head = b.snapshot(None).unwrap().tx;
    let p = view(b, progress(head));
    assert_eq!(p.list(&ListReq::default(), Some(0)).unwrap().value.total, 0);
    assert!(p.ready(&ctx(), &ReadyReq::default(), None).is_err());
}

#[test]
fn ephemeral_and_deferred_items_keep_board_and_ready_semantics() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let ordinary = create(&mut b, "ordinary");
    let ephemeral = engine::create(
        &mut b,
        &ctx(),
        &CreateReq {
            title: "ephemeral".into(),
            ephemeral: true,
            ..Default::default()
        },
    )
    .unwrap()
    .0
    .id;
    let deferred = create(&mut b, "deferred");
    engine::update(
        &mut b,
        &ctx(),
        std::slice::from_ref(&deferred),
        &UpdateReq {
            status: Some("deferred".into()),
            defer: Some("2026-11-01T00:00:00Z".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut all = ListReq {
        all: true,
        deferred: true,
        ..Default::default()
    };
    all.limit = Some(0);
    let expected = engine::list(&b, &all, None).unwrap();
    let head = b.snapshot(None).unwrap().tx;
    let p = view(b, progress(head));
    let board = p.list(&all, Some(1)).unwrap();
    assert_eq!(board.value.issues, expected.issues);
    assert!(board
        .value
        .issues
        .iter()
        .any(|s| s.id == ephemeral && s.ephemeral));
    assert!(board.value.issues.iter().any(|s| s.id == deferred));
    let ready = p.ready(&ctx(), &ReadyReq::default(), Some(1)).unwrap();
    assert_eq!(
        ready.value.issues.iter().map(|s| &s.id).collect::<Vec<_>>(),
        vec![&ordinary]
    );
}
