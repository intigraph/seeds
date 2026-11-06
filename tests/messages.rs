//! Crew mail is written to window graphs and never seen by the board.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used)]

use seeds::backend::{Backend, Claims, Ctx};
use seeds::engine::{self, CreateReq};
use seeds::message::{message_id, window_graph, Message};
use seeds::quipu_backend::QuipuBackend;

const GRAPH: &str = "https://seeds.local/project/sd";

fn ctx(now: &str) -> Ctx {
    Ctx {
        now: now.into(),
        actor: "grant".into(),
        prefix: "sd".into(),
        claims: Claims::default(),
    }
}

fn msg(key: &str, to: &str, at: &str, text: &str) -> Message {
    Message {
        id: message_id(Some(key), to, at, text),
        recipient: to.into(),
        sender: Some("grant".into()),
        sent_at: at.into(),
        text: text.into(),
        kind: "note".into(),
        delivery_key: Some(key.into()),
        about: vec![],
        read_at: None,
    }
}

#[test]
fn messages_land_in_their_week_window_and_never_on_the_board() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let c = ctx("2026-11-02T04:00:00Z");
    engine::create(
        &mut b,
        &c,
        &CreateReq {
            title: "real work".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let before = b.snapshot(None).unwrap();

    let at = "2026-11-02T04:00:00Z";
    let w = window_graph(GRAPH, at).unwrap();
    assert_eq!(w, format!("{GRAPH}/messages/2026-W45"));
    let m1 = msg("k1", "wu", at, "hello wu");
    let m2 = msg("k2", "tim", at, "hello tim");
    assert!(b.message_send(&c, &w, &m1, "st inbox").unwrap().0);
    assert!(b.message_send(&c, &w, &m2, "st inbox").unwrap().0);

    // Idempotent on the id (same delivery key): nothing is rewritten.
    assert!(!b.message_send(&c, &w, &m1, "st inbox").unwrap().0);

    let wu = b.messages(std::slice::from_ref(&w), Some("wu")).unwrap();
    assert_eq!(wu, vec![m1.clone()]);
    assert_eq!(b.messages(std::slice::from_ref(&w), None).unwrap().len(), 2);
    // A window that does not exist holds nothing.
    let empty = window_graph(GRAPH, "2026-10-01T00:00:00Z").unwrap();
    assert!(b.messages(&[empty], None).unwrap().is_empty());

    // The board snapshot is unchanged: no seed, no comment from the mail.
    let after = b.snapshot(None).unwrap();
    assert_eq!(before.seeds.len(), after.seeds.len());
    assert_eq!(before.comments.len(), after.comments.len());
    assert!(after.seeds.values().all(|s| !s.title.contains("hello")));
}

#[test]
fn a_message_outside_a_window_or_malformed_is_refused_and_nothing_written() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let c = ctx("2026-11-02T04:00:00Z");
    let at = "2026-11-02T04:00:00Z";
    let m = msg("k1", "wu", at, "hello");
    for bad in [
        GRAPH.to_string(),
        format!("{GRAPH}#seeds-ephemeral"),
        "https://seeds.local/project/other/messages/2026-W45".to_string(),
    ] {
        assert!(b.message_send(&c, &bad, &m, "st inbox").is_err(), "{bad}");
    }
    let w = window_graph(GRAPH, at).unwrap();
    let mut empty = m.clone();
    empty.text = String::new();
    assert!(b.message_send(&c, &w, &empty, "st inbox").is_err());
    assert!(b.messages(&[w], None).unwrap().is_empty());
}

#[test]
fn a_read_mark_is_written_once_and_listing_reports_it() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let at = "2026-11-02T04:00:00Z";
    let w = window_graph(GRAPH, at).unwrap();
    let m = msg("k1", "wu", at, "hello");
    b.message_send(&ctx(at), &w, &m, "st inbox").unwrap();
    assert!(b.messages(std::slice::from_ref(&w), Some("wu")).unwrap()[0]
        .read_at
        .is_none());
    assert!(b
        .message_mark_read(&ctx("2026-11-02T05:00:00Z"), &w, &m.id)
        .unwrap());
    assert!(!b
        .message_mark_read(&ctx("2026-11-02T06:00:00Z"), &w, &m.id)
        .unwrap());
    let got = b.messages(std::slice::from_ref(&w), Some("wu")).unwrap();
    assert_eq!(got[0].read_at.as_deref(), Some("2026-11-02T05:00:00Z"));
    assert!(b.message_mark_read(&ctx(at), &w, "msg-nope").is_err());
}

#[test]
fn same_key_retry_across_iso_week_keeps_one_original_message() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let first_at = "2026-11-01T23:59:59Z";
    let retry_at = "2026-11-02T00:00:01Z";
    let first_window = window_graph(GRAPH, first_at).unwrap();
    let retry_window = window_graph(GRAPH, retry_at).unwrap();
    assert_ne!(first_window, retry_window);
    let first = msg("week-boundary", "wu", first_at, "one delivery");
    let retry = msg("week-boundary", "wu", retry_at, "one delivery");
    assert_eq!(first.id, retry.id);
    assert!(
        b.message_send(&ctx(first_at), &first_window, &first, "test")
            .unwrap()
            .0
    );
    assert!(
        !b.message_send(&ctx(retry_at), &retry_window, &retry, "test")
            .unwrap()
            .0
    );
    assert_eq!(
        b.messages(&[first_window, retry_window], Some("wu"))
            .unwrap(),
        vec![first]
    );
    assert!(b.snapshot(None).unwrap().seeds.is_empty());
}

#[test]
fn binding_keeps_original_receipt_after_read_and_beyond_recent_windows() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let at = "2026-11-01T23:59:59Z";
    let window = window_graph(GRAPH, at).unwrap();
    let first = msg("retained", "wu", at, "original");
    b.message_send(&ctx(at), &window, &first, "test").unwrap();
    b.message_mark_read(&ctx("2026-11-02T01:00:00Z"), &window, &first.id)
        .unwrap();
    let retry_at = "2027-01-04T00:00:01Z";
    let retry_window = window_graph(GRAPH, retry_at).unwrap();
    let retry = msg("retained", "wu", retry_at, "original");
    assert!(
        !b.message_send(&ctx(retry_at), &retry_window, &retry, "test")
            .unwrap()
            .0
    );
    let (bound, receipt) = b.message_receipt(&first.id).unwrap().unwrap();
    assert_eq!(bound, window);
    assert_eq!(receipt.sent_at, at);
    assert!(receipt.read_at.is_some());
    for changed in [
        msg("retained", "tim", retry_at, "original"),
        msg("retained", "wu", retry_at, "changed"),
    ] {
        assert!(b
            .message_send(&ctx(retry_at), &retry_window, &changed, "test")
            .is_err());
    }
    assert!(b.messages(&[retry_window], None).unwrap().is_empty());
}

#[test]
fn unavailable_original_payload_never_releases_a_delivery_key() {
    use quipu::store::Datum;
    use quipu::types::Op;
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let at = "2026-11-01T23:59:59Z";
    let w = window_graph(GRAPH, at).unwrap();
    let m = msg("unavailable", "wu", at, "retain binding");
    b.message_send(&ctx(at), &w, &m, "test").unwrap();
    let store = b.store_mut();
    let e = store
        .lookup(&seeds::message::message_iri(&m.id))
        .unwrap()
        .unwrap();
    let g = store.lookup(&w).unwrap().unwrap();
    let datums: Vec<_> = store
        .entity_facts_in_graph(e, g)
        .unwrap()
        .into_iter()
        .map(|f| Datum {
            entity: e,
            attribute: f.attribute,
            value: f.value,
            valid_from: at.into(),
            valid_to: None,
            op: Op::Retract,
        })
        .collect();
    store
        .transact_graph_batches(
            &[(g, datums)],
            at,
            Some("test"),
            Some("unavailable payload fixture"),
        )
        .unwrap();
    assert!(b.message_receipt(&m.id).is_err());
    let later = "2027-01-04T00:00:01Z";
    let retry = msg("unavailable", "wu", later, "retain binding");
    let new = window_graph(GRAPH, later).unwrap();
    assert!(b.message_send(&ctx(later), &new, &retry, "test").is_err());
    assert!(b.messages(&[new], None).unwrap().is_empty());
}

#[test]
fn all_live_windows_include_old_unread_and_exclude_other_projects() {
    let mut b = QuipuBackend::in_memory(GRAPH).unwrap();
    let old = msg("old", "wu", "2025-12-22T10:00:00Z", "old unread");
    let new = msg("new", "wu", "2026-11-02T10:00:00Z", "new unread");
    for m in [&old, &new] {
        let w = window_graph(GRAPH, &m.sent_at).unwrap();
        b.message_send(&ctx(&m.sent_at), &w, m, "test").unwrap();
    }
    b.store_mut()
        .graph_create("https://example.org/other/messages/2025-W52")
        .unwrap();
    let recent = seeds::message::recent_windows(GRAPH, &new.sent_at, 2).unwrap();
    assert_eq!(b.messages(&recent, Some("wu")).unwrap(), vec![new.clone()]);
    let live = b.message_windows().unwrap();
    assert_eq!(live.len(), 2);
    assert_eq!(
        b.messages(&live, Some("wu")).unwrap(),
        vec![old.clone(), new]
    );
    let w = window_graph(GRAPH, &old.sent_at).unwrap();
    assert!(b
        .message_mark_read(&ctx("2026-11-02T10:00:00Z"), &w, &old.id)
        .unwrap());
    assert!(b.messages(&live, Some("wu")).unwrap()[0].read_at.is_some());
}
