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
    let five = fill(&mut b, "now", 5, 1);
    let sixth = mk(&mut b, "sixth", 200);
    let err = lane(&mut b, &sixth, "now", 201).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let text = err.to_string();
    assert!(text.contains("lane now is full"), "{text}");
    for id in &five {
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
    let five = fill(&mut b, "now", 5, 1);
    set(
        &mut b,
        &five[0],
        UpdateReq {
            rank: Some("3".into()),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    set(
        &mut b,
        &five[1],
        UpdateReq {
            title: Some("renamed".into()),
            ..Default::default()
        },
        301,
    )
    .unwrap();
    lane(&mut b, &five[2], "now", 302).unwrap(); // same lane again: no-op, not an entry
}

#[test]
fn leaving_frees_a_slot_and_closed_items_do_not_count() {
    let mut b = backend();
    let five = fill(&mut b, "now", 5, 1);
    let sixth = mk(&mut b, "sixth", 200);
    lane(&mut b, &five[0], "later", 201).unwrap();
    lane(&mut b, &sixth, "now", 202).unwrap();
    set(
        &mut b,
        &five[1],
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
    let five = fill(&mut b, "now", 5, 1);
    set(
        &mut b,
        &five[0],
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
        &five[0],
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
    fill(&mut b, "now", 4, 1);
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
    let five = fill(&mut b, "now", 5, 1);
    engine::close(&mut b, &ctx(300), &[five[0].clone()], None, false).unwrap();
    let replacement = mk(&mut b, "replacement", 301);
    lane(&mut b, &replacement, "now", 302).unwrap();
    let err = engine::reopen(&mut b, &ctx(303), &[five[0].clone()], None).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused);
    let status = b
        .snapshot(None)
        .unwrap()
        .get(&five[0])
        .unwrap()
        .status
        .clone();
    assert_eq!(status, "closed", "nothing written");
    // With a free slot, the same reopen goes through.
    lane(&mut b, &replacement, "later", 304).unwrap();
    engine::reopen(&mut b, &ctx(305), &[five[0].clone()], None).unwrap();
}
