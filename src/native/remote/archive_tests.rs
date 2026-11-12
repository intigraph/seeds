//! aegis-n0wc.1: a seed the board archive moved out of the live graph is
//! still found by exact id, an edge to it still carries its status, and its
//! id is never minted again. Runs against a real quipu server; the archive
//! move is the same cross-graph DELETE/INSERT the archive command will send.

use super::tests::quipu_server;
use super::{form_encode, RemoteBackend};
use crate::backend::{Backend, Ctx};
use crate::engine::{self, CreateReq};
use crate::output;
use crate::vocab::{self, term};

fn ctx() -> Ctx {
    Ctx {
        now: "2026-10-07T12:00:00Z".into(),
        actor: "tester".into(),
        prefix: "ar".into(),
        claims: Default::default(),
    }
}

fn create(r: &mut RemoteBackend, c: &Ctx, title: &str, parent: Option<&str>) -> String {
    engine::create(
        r,
        c,
        &CreateReq {
            title: title.into(),
            parent: parent.map(str::to_string),
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0
    .id
}

/// Move `ids` and their comments from the live graph to the archive graph in
/// one update: every fact deleted from live is inserted into the archive.
fn archive(r: &RemoteBackend, ids: &[&str]) {
    let live = r.graph.clone();
    let arch = vocab::archive_graph(&live);
    let items: Vec<String> = ids
        .iter()
        .map(|id| format!("<{}>", vocab::item_iri(id)))
        .collect();
    let items = items.join(" ");
    // VALUES inside each branch: a binding from outside a nested group is not
    // visible inside it, and an unbound ?s there matches the whole graph.
    let update = format!(
        "DELETE {{ GRAPH <{live}> {{ ?s ?p ?o }} }} INSERT {{ GRAPH <{arch}> {{ ?s ?p ?o }} }} \
         WHERE {{ GRAPH <{live}> {{ \
         {{ VALUES ?s {{ {items} }} ?s ?p ?o }} UNION \
         {{ VALUES ?item {{ {items} }} ?s <{}> ?item . ?s ?p ?o }} }} }}",
        term::comment_on()
    );
    let form = format!(
        "update={}&actor=tester&source={}",
        form_encode(&update),
        form_encode("seeds:test-archive")
    );
    r.send("/update", "application/x-www-form-urlencoded", &form, true)
        .expect("archive move");
}

/// The move took exactly `moved` out of the live graph: every other seed in
/// `kept` is still live (the control that the move is not graph-wide).
fn assert_moved_only(r: &RemoteBackend, moved: &[&str], kept: &[&str]) {
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let live = r.snapshot_items(&ids(kept)).unwrap();
    for id in kept {
        assert!(live.seeds.contains_key(*id), "{id} must stay live");
    }
    assert!(
        r.snapshot_items(&ids(moved)).unwrap().seeds.is_empty(),
        "moved ids left live"
    );
}

fn server_or_skip() -> Option<super::tests::Server> {
    let server = quipu_server();
    if server.is_none() {
        eprintln!("SKIPPED: set SEEDS_TEST_QUIPU_SERVER to a quipu-server binary to run this");
    }
    server
}

#[test]
fn an_archived_seed_is_shown_by_id_with_its_comments() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-show",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let old = create(&mut r, &c, "old closed work", None);
    engine::comment_add(&mut r, &c, &old, "the evidence", None).unwrap();
    engine::close(&mut r, &c, std::slice::from_ref(&old), Some("done"), false).unwrap();
    archive(&r, &[&old]);

    // Control: the live graph no longer holds it, so a live-only read misses it.
    assert!(r
        .snapshot_items(std::slice::from_ref(&old))
        .unwrap()
        .seeds
        .is_empty());

    let views = engine::show(&r, std::slice::from_ref(&old), None).unwrap();
    assert_eq!(views[0].seed.status, "closed");
    assert_eq!(views[0].comments.len(), 1, "its comments moved with it");
    let json = output::show_json(&views);
    assert_eq!(json[0]["archived"], serde_json::json!([old]));
    assert!(output::view_text(&views[0]).contains("archived (read from the board archive)"));
}

#[test]
fn an_edge_to_an_archived_blocker_keeps_its_closed_status() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-edge",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let blocker = create(&mut r, &c, "blocker", None);
    let dependent = create(&mut r, &c, "dependent", None);
    engine::dep_add(&mut r, &c, &dependent, &blocker, "blocks").unwrap();
    engine::close(
        &mut r,
        &c,
        std::slice::from_ref(&blocker),
        Some("done"),
        false,
    )
    .unwrap();
    archive(&r, &[&blocker]);

    let views = engine::show(&r, std::slice::from_ref(&dependent), None).unwrap();
    let json = output::show_json(&views);
    let edge = &json[0]["dependencies"][0];
    assert_eq!(edge["id"], serde_json::json!(blocker));
    assert_eq!(
        edge["status"],
        serde_json::json!("closed"),
        "never null for an archived blocker"
    );
    assert_eq!(json[0]["archived"], serde_json::json!([blocker]));
    // Shown by its own id, the archived blocker reports itself.
    let live = engine::show(&r, std::slice::from_ref(&blocker), None).unwrap();
    assert_eq!(
        output::show_json(&live)[0]["archived"],
        serde_json::json!([blocker])
    );
}

#[test]
fn an_unarchived_board_shows_no_archived_ids() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-none",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let a = create(&mut r, &c, "a", None);
    let b = create(&mut r, &c, "b", None);
    engine::dep_add(&mut r, &c, &a, &b, "blocks").unwrap();
    let json = output::show_json(&engine::show(&r, std::slice::from_ref(&a), None).unwrap());
    assert_eq!(json[0]["archived"], serde_json::json!([]));
    assert_eq!(
        json[0]["dependencies"][0]["status"],
        serde_json::json!("open")
    );
    let missing = engine::show(&r, &["ar-nope".to_string()], None).unwrap_err();
    assert_eq!(
        missing.kind,
        crate::error::ErrorKind::NotFound,
        "absent everywhere is still NOT_FOUND"
    );
}

#[test]
fn an_archived_child_id_is_never_minted_again() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-mint",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let parent = create(&mut r, &c, "epic", None);
    let first = create(&mut r, &c, "first child", Some(&parent));
    engine::close(
        &mut r,
        &c,
        std::slice::from_ref(&first),
        Some("done"),
        false,
    )
    .unwrap();
    archive(&r, &[&first]);
    assert_moved_only(&r, &[&first], &[&parent]);
    let second = create(&mut r, &c, "second child", Some(&parent));
    assert_ne!(second, first, "the archived child still owns its id");
    // Both are still readable by id, one live and one archived.
    let views = engine::show(&r, &[first.clone(), second.clone()], None).unwrap();
    assert_eq!(views[0].seed.title, "first child");
    assert_eq!(views[1].seed.title, "second child");
}

#[test]
fn a_write_to_an_archived_seed_is_refused_and_leaves_no_live_stub() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-write",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let old = create(&mut r, &c, "archived work", None);
    let live = create(&mut r, &c, "live work", None);
    engine::close(&mut r, &c, std::slice::from_ref(&old), Some("done"), false).unwrap();
    archive(&r, &[&old]);
    assert_moved_only(&r, &[&old], &[&live]);

    let comment = engine::comment_add(&mut r, &c, &old, "late note", None);
    let reclose = engine::close(&mut r, &c, std::slice::from_ref(&old), Some("again"), false);
    let dep = engine::dep_add(&mut r, &c, &live, &old, "blocks");
    for (what, result) in [
        ("comment", comment.map(|_| ())),
        ("close", reclose.map(|_| ())),
        ("dep add onto it", dep.map(|_| ())),
    ] {
        assert!(
            result.is_err(),
            "{what} on an archived seed must be refused"
        );
    }
    assert!(
        r.snapshot_items(std::slice::from_ref(&old))
            .unwrap()
            .seeds
            .is_empty(),
        "no write recreated the archived seed in the live graph"
    );
    let shown = engine::show(&r, std::slice::from_ref(&old), None).unwrap();
    assert_eq!(
        shown[0].comments.len(),
        0,
        "the refused comment did not land anywhere"
    );
    assert_eq!(shown[0].seed.close_reason.as_deref(), Some("done"));
}
