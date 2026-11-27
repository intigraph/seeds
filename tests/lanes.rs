//! aegis-6vto.4: a work lane beside rank, with caps checked on write
//! (Now 5, Next 40). Against an in-memory quipu store, like tests/core.rs.
#![allow(clippy::unwrap_used)]

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, Filter, ListReq, UpdateReq};
use seeds::error::ErrorKind;
use seeds::quipu_backend::QuipuBackend;

const GRAPH: &str = "https://seeds.local/project/sd";

fn backend() -> QuipuBackend {
    QuipuBackend::in_memory(GRAPH).unwrap()
}

fn ctx(n: u32) -> Ctx {
    Ctx {
        now: format!("2026-09-30T00:{:02}:{:02}Z", (n / 60) % 60, n % 60),
        actor: "tester".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}

fn mk(b: &mut QuipuBackend, title: &str, n: u32) -> String {
    engine::create(
        b,
        &ctx(n),
        &CreateReq {
            title: title.into(),
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0
    .id
}

fn set(b: &mut QuipuBackend, id: &str, req: UpdateReq, n: u32) -> seeds::error::Result<()> {
    engine::update(b, &ctx(n), &[id.to_string()], &req).map(|_| ())
}

fn lane(b: &mut QuipuBackend, id: &str, l: &str, n: u32) -> seeds::error::Result<()> {
    set(
        b,
        id,
        UpdateReq {
            lane: Some(l.into()),
            ..Default::default()
        },
        n,
    )
}

fn stored_lane(b: &QuipuBackend, id: &str) -> Option<String> {
    b.snapshot(None).unwrap().get(id).unwrap().lane.clone()
}

/// The Now cap under test, read from the model so a cap change needs no test edit.
fn now_cap() -> usize {
    seeds::model::lane_cap("now").expect("now is capped")
}

fn fill(b: &mut QuipuBackend, l: &str, count: usize, start: u32) -> Vec<String> {
    (0..count)
        .map(|i| {
            let id = mk(b, &format!("{l} {i}"), start + i as u32);
            lane(b, &id, l, start + 100 + i as u32).unwrap();
            id
        })
        .collect()
}

#[test]
fn lane_round_trips_and_clears() {
    let mut b = backend();
    let id = mk(&mut b, "x", 1);
    lane(&mut b, &id, "Next", 2).unwrap();
    assert_eq!(
        stored_lane(&b, &id).as_deref(),
        Some("next"),
        "stored lowercase"
    );
    lane(&mut b, &id, "", 3).unwrap();
    assert_eq!(stored_lane(&b, &id), None);
}

#[test]
fn an_unknown_lane_is_a_usage_error_and_writes_nothing() {
    let mut b = backend();
    let id = mk(&mut b, "x", 1);
    let err = lane(&mut b, &id, "soon", 2).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Usage);
    assert_eq!(stored_lane(&b, &id), None);
}

#[test]
fn a_sixth_now_item_is_refused_and_names_what_could_leave() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    let sixth = mk(&mut b, "sixth", 200);
    let err = lane(&mut b, &sixth, "now", 201).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let text = err.to_string();
    assert!(text.contains("lane now is full"), "{text}");
    for id in &full {
        assert!(text.contains(id.as_str()), "{id} missing from {text}");
    }
    assert_eq!(stored_lane(&b, &sixth), None, "nothing written");
}

#[test]
fn the_forty_first_next_item_is_refused_but_later_is_uncapped() {
    let mut b = backend();
    fill(&mut b, "next", 40, 1);
    let extra = mk(&mut b, "41st", 500);
    assert_eq!(
        lane(&mut b, &extra, "next", 501).unwrap_err().kind,
        ErrorKind::Refused
    );
    lane(&mut b, &extra, "later", 502).unwrap();
    fill(&mut b, "later", 45, 600);
}

#[test]
fn editing_or_reranking_an_item_already_in_a_full_lane_is_not_refused() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    set(
        &mut b,
        &full[0],
        UpdateReq {
            rank: Some("3".into()),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    set(
        &mut b,
        &full[1],
        UpdateReq {
            title: Some("renamed".into()),
            ..Default::default()
        },
        301,
    )
    .unwrap();
    lane(&mut b, &full[2], "now", 302).unwrap(); // same lane again: no-op, not an entry
}

#[test]
fn leaving_frees_a_slot_and_closed_items_do_not_count() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    let sixth = mk(&mut b, "sixth", 200);
    lane(&mut b, &full[0], "later", 201).unwrap();
    lane(&mut b, &sixth, "now", 202).unwrap();
    set(
        &mut b,
        &full[1],
        UpdateReq {
            status: Some("closed".into()),
            ..Default::default()
        },
        203,
    )
    .unwrap();
    let seventh = mk(&mut b, "seventh", 204);
    lane(&mut b, &seventh, "now", 205).unwrap();
}

#[test]
fn reopening_a_closed_item_inside_a_full_lane_is_an_entry_and_is_refused() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    set(
        &mut b,
        &full[0],
        UpdateReq {
            status: Some("closed".into()),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let replacement = mk(&mut b, "replacement", 301);
    lane(&mut b, &replacement, "now", 302).unwrap();
    let err = set(
        &mut b,
        &full[0],
        UpdateReq {
            status: Some("open".into()),
            ..Default::default()
        },
        303,
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
}

#[test]
fn moving_several_items_in_one_write_counts_them_all() {
    let mut b = backend();
    fill(&mut b, "now", now_cap() - 1, 1); // one place left; two entries overflow
    let a = mk(&mut b, "a", 200);
    let c = mk(&mut b, "c", 201);
    let err = engine::update(
        &mut b,
        &ctx(202),
        &[a.clone(), c.clone()],
        &UpdateReq {
            lane: Some("now".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    assert_eq!(stored_lane(&b, &a), None);
    assert_eq!(stored_lane(&b, &c), None);
}

#[test]
fn list_filters_by_lane() {
    let mut b = backend();
    let now = fill(&mut b, "now", 2, 1);
    fill(&mut b, "next", 3, 50);
    mk(&mut b, "unlaned", 99);
    let req = ListReq {
        filter: Filter {
            lane: Some("now".into()),
            ..Default::default()
        },
        limit: Some(0),
        ..Default::default()
    };
    let page = engine::list(&b, &req, None).unwrap();
    let mut got: Vec<String> = page.issues.iter().map(|s| s.id.clone()).collect();
    got.sort();
    let mut want = now.clone();
    want.sort();
    assert_eq!(got, want);
}

// ian's #264 review: `sd reopen` commits through transition(), which used to
// skip the cap, so reopening a closed Now item could make a sixth.
#[test]
fn sd_reopen_into_a_full_lane_is_refused_and_writes_nothing() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    engine::close(&mut b, &ctx(300), &[full[0].clone()], None, false).unwrap();
    let replacement = mk(&mut b, "replacement", 301);
    lane(&mut b, &replacement, "now", 302).unwrap();
    let err = engine::reopen(&mut b, &ctx(303), &[full[0].clone()], None).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let status = b
        .snapshot(None)
        .unwrap()
        .get(&full[0])
        .unwrap()
        .status
        .clone();
    assert_eq!(status, "closed", "nothing written");
    // With a free slot, the same reopen goes through.
    lane(&mut b, &replacement, "later", 304).unwrap();
    engine::reopen(&mut b, &ctx(305), &[full[0].clone()], None).unwrap();
}

// ---------------------------------------------------------------- aegis-8dda.3

fn ice(
    b: &mut QuipuBackend,
    id: &str,
    reason: &str,
    review: &str,
    n: u32,
) -> seeds::error::Result<()> {
    set(
        b,
        id,
        UpdateReq {
            lane: Some("icebox".into()),
            icebox_reason: Some(reason.into()),
            icebox_review: Some(review.into()),
            ..Default::default()
        },
        n,
    )
}

fn assign(b: &mut QuipuBackend, id: &str, who: &str, l: &str, n: u32) -> seeds::error::Result<()> {
    set(
        b,
        id,
        UpdateReq {
            lane: Some(l.into()),
            assignee: Some(who.into()),
            ..Default::default()
        },
        n,
    )
}

/// Assert `laneCap` literals on the project graph's board node.
fn board_caps(b: &mut QuipuBackend, facts: &[&str]) {
    use quipu::{Datum, Op, Value};
    let store = b.store_mut();
    let graph = store.graph_create(GRAPH).unwrap();
    let entity = store.intern(&seeds::vocab::board_iri(GRAPH)).unwrap();
    let attribute = store.intern(&seeds::vocab::term::lane_cap()).unwrap();
    let datums: Vec<Datum> = facts
        .iter()
        .map(|f| Datum {
            entity,
            attribute,
            value: Value::Str((*f).into()),
            valid_from: "2026-09-30T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store
        .transact_to_graph(&datums, "2026-09-30T00:00:00Z", None, None, graph)
        .unwrap();
}

#[test]
fn triage_and_icebox_are_lanes() {
    let mut b = backend();
    let id = mk(&mut b, "x", 1);
    lane(&mut b, &id, "Triage", 2).unwrap();
    assert_eq!(stored_lane(&b, &id).as_deref(), Some("triage"));
    ice(&mut b, &id, "waiting on quipu v3", "2027-01-15", 3).unwrap();
    let s = b.snapshot(None).unwrap().get(&id).unwrap().clone();
    assert_eq!(s.lane.as_deref(), Some("icebox"));
    assert_eq!(s.icebox_reason.as_deref(), Some("waiting on quipu v3"));
    assert_eq!(s.icebox_review.as_deref(), Some("2027-01-15"));
    // Leaving the icebox clears its reason and review.
    lane(&mut b, &id, "later", 4).unwrap();
    let s = b.snapshot(None).unwrap().get(&id).unwrap().clone();
    assert_eq!((s.icebox_reason, s.icebox_review), (None, None));
}

#[test]
fn the_icebox_needs_a_reason_and_a_review_date_or_event() {
    let mut b = backend();
    let id = mk(&mut b, "x", 1);
    assert_eq!(
        lane(&mut b, &id, "icebox", 2).unwrap_err().kind,
        ErrorKind::Usage
    );
    assert_eq!(
        ice(&mut b, &id, "why", "", 3).unwrap_err().kind,
        ErrorKind::Usage
    );
    assert_eq!(
        ice(&mut b, &id, " ", "when v3 ships", 4).unwrap_err().kind,
        ErrorKind::Usage
    );
    assert_eq!(stored_lane(&b, &id), None, "nothing written");
    ice(&mut b, &id, "why", "when v3 ships", 5).unwrap();
}

#[test]
fn the_icebox_is_excluded_from_counts() {
    let mut b = backend();
    let full = fill(&mut b, "now", now_cap(), 1);
    // Parking a Now item frees its slot; the icebox itself is never capped.
    ice(&mut b, &full[0], "parked", "next sprint", 300).unwrap();
    let extra = mk(&mut b, "extra", 301);
    lane(&mut b, &extra, "now", 302).unwrap();
    for i in 0..now_cap() + 5 {
        let id = mk(&mut b, &format!("ice {i}"), 400 + i as u32);
        ice(&mut b, &id, "parked", "2027-03-01", 500 + i as u32).unwrap();
    }
}

#[test]
fn now_holds_two_per_assignee_and_refuses_the_third_with_dispositions() {
    let mut b = backend();
    let a = mk(&mut b, "a", 1);
    let c = mk(&mut b, "c", 2);
    let d = mk(&mut b, "d", 3);
    assign(&mut b, &a, "wu", "now", 4).unwrap();
    assign(&mut b, &c, "wu", "now", 5).unwrap();
    let err = assign(&mut b, &d, "wu", "now", 6).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let text = err.to_string();
    assert!(text.contains("lane now for wu is full"), "{text}");
    assert!(
        text.contains("duplicate-of") && text.contains("icebox"),
        "{text}"
    );
    assert!(text.contains(&a) && text.contains(&c), "{text}");
    assert_eq!(stored_lane(&b, &d), None, "nothing written");
    // Another crew member has their own two slots.
    assign(&mut b, &d, "ian", "now", 7).unwrap();
    // Reassigning within Now enters the new assignee's slots.
    let e = mk(&mut b, "e", 8);
    assign(&mut b, &e, "ian", "now", 9).unwrap();
    assert_eq!(
        set(
            &mut b,
            &a,
            UpdateReq {
                assignee: Some("ian".into()),
                ..Default::default()
            },
            10
        )
        .unwrap_err()
        .kind,
        ErrorKind::Refused
    );
}

#[test]
fn later_is_capped_at_150_by_default_and_the_hint_names_dispositions() {
    let mut b = backend();
    assert_eq!(seeds::model::lane_cap("later"), Some(150));
    board_caps(&mut b, &["later=3"]);
    fill(&mut b, "later", 3, 1);
    let extra = mk(&mut b, "fourth", 200);
    let err = lane(&mut b, &extra, "later", 201).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let text = err.to_string();
    assert!(
        text.contains("lane later is full") && text.contains("disposition"),
        "{text}"
    );
    // Triage is uncapped by default: unrefined intake always has a place.
    lane(&mut b, &extra, "triage", 202).unwrap();
}

#[test]
fn caps_are_data_from_the_board_node_and_zero_uncaps() {
    let mut b = backend();
    board_caps(&mut b, &["next=0", "triage=1"]);
    fill(&mut b, "next", 41, 1);
    fill(&mut b, "triage", 1, 300);
    let extra = mk(&mut b, "x", 400);
    assert_eq!(
        lane(&mut b, &extra, "triage", 401).unwrap_err().kind,
        ErrorKind::Refused
    );
}

#[test]
fn caps_come_from_config_and_must_agree_with_the_graph() {
    let mut b = backend();
    let mut cfg = seeds::model::LaneCapSettings::new();
    cfg.insert("now_per_assignee".into(), 1);
    b.set_lane_cap_config(cfg.clone());
    let a = mk(&mut b, "a", 1);
    let c = mk(&mut b, "c", 2);
    assign(&mut b, &a, "wu", "now", 3).unwrap();
    assert_eq!(
        assign(&mut b, &c, "wu", "now", 4).unwrap_err().kind,
        ErrorKind::Refused
    );
    // The same value in both sources agrees; a different one is a conflict
    // that writes nothing and picks no side.
    board_caps(&mut b, &["now_per_assignee=1"]);
    assert_eq!(
        assign(&mut b, &c, "wu", "now", 5).unwrap_err().kind,
        ErrorKind::Refused
    );
    let mut b = backend();
    b.set_lane_cap_config(cfg);
    board_caps(&mut b, &["now_per_assignee=3"]);
    let a = mk(&mut b, "a", 1);
    let err = assign(&mut b, &a, "wu", "now", 2).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Conflict);
    assert!(err.to_string().contains("now_per_assignee"), "{err}");
    assert_eq!(stored_lane(&b, &a), None, "nothing written");
}
