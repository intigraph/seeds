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

/// Review of #290, finding 2: the ephemeral graph. An ephemeral seed is never
/// ready, and a durable seed whose blocker is ephemeral reads exactly as the
/// ready definition (project graph only) and the whole snapshot read it.
#[test]
fn scoped_ready_matches_the_whole_snapshot_across_the_ephemeral_graph() {
    let (mut b, ids) = board();
    let eph = engine::create(
        &mut b,
        &ctx(30),
        &CreateReq {
            title: "ephemeral".into(),
            ephemeral: true,
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0
    .id;
    let lone = engine::create(
        &mut b,
        &ctx(31),
        &CreateReq {
            title: "ephemeral alone".into(),
            ephemeral: true,
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0
    .id;
    // A shared seed cannot be blocked by an ephemeral one (the engine refuses
    // the edge), so a durable blocker-status lookup never needs that graph.
    assert!(engine::dep_add(&mut b, &ctx(32), &ids[0], &eph, "blocks").is_err());
    // Ephemeral seeds blocked by an ephemeral and by a durable open seed.
    engine::dep_add(&mut b, &ctx(33), &lone, &eph, "blocks").unwrap();
    engine::dep_add(&mut b, &ctx(34), &eph, &ids[2], "blocks").unwrap();
    assert_eq!(b.ready_ids(None).unwrap(), b.ready_ids(head(&b)).unwrap());
    for include_deferred in [false, true] {
        let req = ReadyReq {
            include_deferred,
            ..ReadyReq::default()
        };
        let scoped = engine::ready(&b, &ctx(40), &req, None).unwrap();
        let whole = engine::ready(&b, &ctx(40), &req, head(&b)).unwrap();
        assert_eq!(
            format!("{:?}", scoped.issues),
            format!("{:?}", whole.issues)
        );
        let got: Vec<&str> = scoped.issues.iter().map(|s| s.id.as_str()).collect();
        assert!(
            !got.contains(&eph.as_str()) && !got.contains(&lone.as_str()),
            "{got:?}"
        );
    }
    for id in [&ids[0], &eph, &lone] {
        let scoped = engine::show(&b, std::slice::from_ref(id), None).unwrap();
        let whole = engine::show(&b, std::slice::from_ref(id), head(&b)).unwrap();
        assert_eq!(format!("{scoped:?}"), format!("{whole:?}"), "{id}");
    }
    let req = ListReq {
        all: true,
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

/// Finding 4: the edge read is restricted to open items. Closed, in-progress,
/// deferred and tombstone seeds blocked by open seeds, and open seeds blocked
/// by in-progress or tombstone seeds, still read as the definition reads them.
#[test]
fn scoped_ready_ignores_blocked_history_exactly_as_the_definition_does() {
    let (mut b, ids) = board();
    let more: Vec<String> = (50..56).map(|i| mk(&mut b, &format!("x {i}"), i)).collect();
    // closed seed blocked by an open one
    engine::dep_add(&mut b, &ctx(60), &more[0], &ids[0], "blocks").unwrap();
    engine::close(&mut b, &ctx(61), &[more[0].clone()], Some("done"), true).unwrap();
    // open seed blocked by an in-progress one
    engine::dep_add(&mut b, &ctx(62), &more[1], &more[2], "blocks").unwrap();
    engine::update(
        &mut b,
        &ctx(63),
        std::slice::from_ref(&more[2]),
        &engine::UpdateReq {
            status: Some("in_progress".into()),
            ..engine::UpdateReq::default()
        },
    )
    .unwrap();
    // open seed blocked by a tombstone
    engine::dep_add(&mut b, &ctx(64), &more[3], &more[4], "blocks").unwrap();
    engine::delete(
        &mut b,
        &ctx(65),
        &[more[4].clone()],
        "gone",
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(b.ready_ids(None).unwrap(), b.ready_ids(head(&b)).unwrap());
    let req = ReadyReq::default();
    let scoped = engine::ready(&b, &ctx(70), &req, None).unwrap();
    let whole = engine::ready(&b, &ctx(70), &req, head(&b)).unwrap();
    assert_eq!(
        format!("{:?}", scoped.issues),
        format!("{:?}", whole.issues)
    );
    let got: Vec<&str> = scoped.issues.iter().map(|s| s.id.as_str()).collect();
    assert!(
        !got.contains(&more[0].as_str()),
        "closed is never ready: {got:?}"
    );
    assert!(
        !got.contains(&more[1].as_str()),
        "in-progress blocks: {got:?}"
    );
    assert!(
        got.contains(&more[3].as_str()),
        "tombstone does not block: {got:?}"
    );
}

/// Finding 3: multi-id `show` keeps every comment (it reads through
/// `snapshot_items`; `snapshot_seeds` may omit comments by contract).
#[test]
fn scoped_multi_id_show_keeps_comments() {
    let (b, ids) = board();
    let scoped = engine::show(&b, &ids, None).unwrap();
    let whole = engine::show(&b, &ids, head(&b)).unwrap();
    assert_eq!(format!("{scoped:?}"), format!("{whole:?}"));
    let text = format!("{scoped:?}");
    assert!(text.contains("first") && text.contains("second"), "{text}");
}

/// Finding 1: ids that would break an IRI term are percent-encoded. `show`
/// refuses them exactly as a whole snapshot does, and a dependents read of
/// them answers nothing rather than failing to parse.
#[test]
fn malformed_ids_read_as_missing_never_as_sparql() {
    let (b, ids) = board();
    let bad: Vec<String> = [
        "bad>id",
        "foo bar",
        "a{b}|c",
        "x\"y",
        "<http://e/x> } ?s ?p ?o {",
        "nl\nid",
        "back\\slash^`",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for id in &bad {
        let scoped = engine::show(&b, std::slice::from_ref(id), None);
        let whole = engine::show(&b, std::slice::from_ref(id), head(&b));
        assert_eq!(format!("{scoped:?}"), format!("{whole:?}"), "{id:?}");
        assert!(scoped.is_err(), "{id:?}");
    }
    let deps = b.snapshot_dependents(&bad).unwrap();
    assert!(deps.seeds.is_empty(), "{:?}", deps.seeds.keys());
    // mixed with real ids, the real dependents are still found
    let mut mixed = bad.clone();
    mixed.push(ids[0].clone());
    let deps = b.snapshot_dependents(&mixed).unwrap();
    assert!(deps.seeds.contains_key(&ids[7]) && deps.seeds.contains_key(&ids[8]));
}

/// Finding 5: dependents are read in deduplicated chunks; a listing larger
/// than one chunk (and repeated ids) gives the whole snapshot's counts.
#[test]
fn chunked_dependents_match_the_whole_snapshot() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let ids: Vec<String> = (0..90).map(|i| mk(&mut b, &format!("s {i}"), i)).collect();
    for i in 1..90 {
        let kind = ["blocks", "parent-child", "related", "discovered-from"][i % 4];
        engine::dep_add(&mut b, &ctx(100 + i as u32), &ids[i], &ids[i / 2], kind).unwrap();
    }
    let req = ListReq {
        all: true,
        limit: Some(0),
        ..ListReq::default()
    };
    let scoped = engine::list(&b, &req, None).unwrap();
    let whole = engine::list(&b, &req, head(&b)).unwrap();
    assert_eq!(
        format!("{:?}", scoped.issues),
        format!("{:?}", whole.issues)
    );
    assert_eq!(scoped.dependent_counts, whole.dependent_counts);
    let mut doubled = ids.clone();
    doubled.extend(ids.iter().cloned());
    let once = b.snapshot_dependents(&ids).unwrap();
    let twice = b.snapshot_dependents(&doubled).unwrap();
    assert_eq!(
        once.seeds.keys().collect::<Vec<_>>(),
        twice.seeds.keys().collect::<Vec<_>>()
    );
    assert_eq!(once.seeds.len(), 89);
}

/// Non-blocking note (c): filtered listings through `snapshot_where`.
#[test]
fn scoped_filtered_lists_print_what_a_whole_snapshot_prints() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    for i in 0..12u32 {
        let req = CreateReq {
            title: format!("f {i}"),
            assignee: (i % 3 == 0).then(|| "ann".to_string()),
            priority: Some(format!("{}", i % 4)),
            labels: if i % 2 == 0 {
                vec!["red".into()]
            } else {
                vec![]
            },
            issue_type: (i % 5 == 0).then(|| "bug".to_string()),
            ..CreateReq::default()
        };
        engine::create(&mut b, &ctx(i), &req).unwrap();
    }
    let filters = [
        engine::Filter {
            assignee: Some("ann".into()),
            ..Default::default()
        },
        engine::Filter {
            labels: vec!["red".into()],
            ..Default::default()
        },
        engine::Filter {
            priority: Some("2".into()),
            ..Default::default()
        },
        engine::Filter {
            issue_type: Some("bug".into()),
            ..Default::default()
        },
        engine::Filter {
            status: Some("open".into()),
            ..Default::default()
        },
    ];
    for filter in filters {
        let req = ListReq {
            filter: filter.clone(),
            all: true,
            ..ListReq::default()
        };
        let scoped = engine::list(&b, &req, None).unwrap();
        let whole = engine::list(&b, &req, head(&b)).unwrap();
        assert_eq!(
            format!("{:?}", scoped.issues),
            format!("{:?}", whole.issues),
            "{filter:?}"
        );
    }
}
