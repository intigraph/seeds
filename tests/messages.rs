//! Crew mail is written to window graphs and never seen by the board (aegis-snq).
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
