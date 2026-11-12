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

// ------------------------------------------------------------ the move itself

use super::archive::{resume_ids, Protect};

#[test]
fn resume_conditions_name_their_closed_targets() {
    let notes = "text\nresume_when: closed:aegis-q94\nhistorical resume condition: closed:old-1\n  resume_when: closed:a.1, closed:b-2;pr-merged:x";
    let ids: Vec<String> = resume_ids(notes).into_iter().collect();
    assert_eq!(
        ids,
        vec!["a.1", "aegis-q94", "b-2"],
        "historical lines are not live conditions"
    );
}

#[test]
fn a_protect_file_is_parsed_strictly() {
    let p = Protect::parse(r#"{"items":[{"id":"x-1","reason":"r","source":"s"}],"protected_labels":["pin:never-archive"],"protected_label_prefixes":["recurring-class:"]}"#).unwrap();
    assert!(p.ids.contains("x-1"));
    assert!(p.labels.contains("pin:never-archive"));
    assert_eq!(p.label_prefixes, vec!["recurring-class:"]);
    assert!(Protect::parse(r#"{"items":[{"reason":"no id"}]}"#).is_err());
    assert!(Protect::parse("not json").is_err());
}

#[test]
fn selection_keeps_every_protected_seed_live() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-select",
        None,
        None,
        &[],
    )
    .unwrap();
    let mut c = ctx();
    c.now = "2026-09-01T00:00:00Z".into();
    let mk = |r: &mut RemoteBackend, c: &Ctx, title: &str| create(r, c, title, None);
    let plain = mk(&mut r, &c, "plain old close");
    let listed = mk(&mut r, &c, "listed control");
    let blocker = mk(&mut r, &c, "blocker of open work");
    let gate = mk(&mut r, &c, "named in a resume condition");
    let pinned = mk(&mut r, &c, "pinned by label");
    let open = mk(&mut r, &c, "open work");
    let deferred = mk(&mut r, &c, "deferred work");
    engine::dep_add(&mut r, &c, &open, &blocker, "blocks").unwrap();
    let u = engine::UpdateReq {
        notes: Some(format!(
            "waiting
resume_when: closed:{gate}"
        )),
        status: Some("deferred".into()),
        ..Default::default()
    };
    engine::update(&mut r, &c, std::slice::from_ref(&deferred), &u).unwrap();
    let l = engine::UpdateReq {
        add_labels: vec!["recurring-class:abc".into()],
        ..Default::default()
    };
    engine::update(&mut r, &c, std::slice::from_ref(&pinned), &l).unwrap();
    for id in [&plain, &listed, &blocker, &gate, &pinned] {
        engine::close(&mut r, &c, std::slice::from_ref(id), Some("done"), false).unwrap();
    }
    let protect = Protect {
        ids: [listed.clone()].into(),
        label_prefixes: vec!["recurring-class:".into()],
        ..Protect::default()
    };
    let cutoff = "2026-10-01T00:00:00Z";
    let protected = r.archive_protected(cutoff, &protect).unwrap();
    for id in [&listed, &blocker, &gate, &pinned] {
        assert!(
            protected.contains_key(id),
            "{id} must be protected: {protected:?}"
        );
    }
    assert!(!protected.contains_key(&plain));
    let candidates = r.archive_candidates(cutoff, "", 100).unwrap();
    let movable: Vec<&String> = candidates
        .iter()
        .filter(|id| !protected.contains_key(*id))
        .collect();
    assert_eq!(
        movable,
        vec![&plain],
        "only the unprotected old close moves"
    );
    // A cutoff before every close selects nothing.
    assert!(r
        .archive_candidates("2026-08-01T00:00:00Z", "", 100)
        .unwrap()
        .is_empty());
}

#[test]
fn archive_then_unarchive_restores_every_fact_byte_for_byte() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-roundtrip",
        None,
        None,
        &[],
    )
    .unwrap();
    let c = ctx();
    let a = create(&mut r, &c, "a", None);
    let b = create(&mut r, &c, "b", None);
    let reopened = create(&mut r, &c, "reopened", None);
    let keep = create(&mut r, &c, "keep", None);
    engine::comment_add(&mut r, &c, &a, "one", None).unwrap();
    engine::comment_add(&mut r, &c, &a, "two", None).unwrap();
    for id in [&a, &b] {
        engine::close(&mut r, &c, std::slice::from_ref(id), Some("done"), false).unwrap();
    }
    let ids = vec![a.clone(), b.clone(), reopened.clone()];
    let before = r.family_facts_for_test(&r.graph.clone(), &ids);

    let moved = r.archive_move(&c, &ids, true).unwrap();
    let mut expect = vec![a.clone(), b.clone()];
    expect.sort();
    assert_eq!(moved.ids, expect, "an open seed is never moved");
    assert_eq!(moved.subjects, 4, "two seeds and two comments");
    assert_moved_only(&r, &[&a, &b], &[&reopened, &keep]);
    let shown = engine::show(&r, std::slice::from_ref(&a), None).unwrap();
    assert_eq!(shown[0].comments.len(), 2);

    assert_eq!(
        r.archived_count().unwrap(),
        Some(2),
        "stats counts the archive explicitly"
    );
    let st = engine::stats(&r, &c, engine::StatsReq::default(), None).unwrap();
    assert_eq!(st.archived, Some(2));
    assert_eq!(
        st.closed, 0,
        "the live closed count no longer includes archived seeds"
    );
    let back = r.archive_move(&c, &[a.clone(), b.clone()], false).unwrap();
    assert_eq!(back.subjects, 4);
    let after = r.family_facts_for_test(&r.graph.clone(), &ids);
    assert_eq!(before, after, "restore is byte-equal per subject");
    assert!(
        r.family_facts_for_test(&vocab::archive_graph(&r.graph), &ids)
            .is_empty(),
        "nothing left in the archive"
    );
    assert_eq!(r.archived_count().unwrap(), Some(0));
}

/// ian review of #266: an update can commit and still fail verification. The
/// batch's intent line is logged BEFORE the update, the error names its ids,
/// and `unarchive --from-log` restores it.
#[test]
fn a_batch_that_fails_verification_after_commit_is_logged_and_reversible() {
    let Some(server) = server_or_skip() else {
        return;
    };
    let mut r = RemoteBackend::connect(
        &server.base,
        "https://seeds.local/project/archive-fault",
        None,
        None,
        &[],
    )
    .unwrap();
    let mut c = ctx();
    c.now = "2026-09-01T00:00:00Z".into();
    let a = create(&mut r, &c, "a", None);
    engine::comment_add(&mut r, &c, &a, "one", None).unwrap();
    engine::close(&mut r, &c, std::slice::from_ref(&a), Some("done"), false).unwrap();
    let ids = vec![a.clone()];
    let live = r.graph.clone();
    let before = r.family_facts_for_test(&live, &ids);

    let dir =
        std::env::temp_dir().join(format!("seeds-archive-fault-{}-{}", std::process::id(), a));
    std::fs::create_dir_all(&dir).unwrap();
    let protect = dir.join("protect.json");
    std::fs::write(&protect, r#"{"items":[]}"#).unwrap();
    let log = dir.join("log.jsonl");

    // A fact added to the seed in the gap between the pre-read and the update:
    // the update commits and moves it too, so the read-back cannot match.
    let injected = "https://seeds.local/test#injected";
    let (g, item) = (live.clone(), vocab::item_iri(&a));
    super::archive::BEFORE_UPDATE.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |r: &RemoteBackend| {
            let update =
                format!("INSERT DATA {{ GRAPH <{g}> {{ <{item}> <{injected}> \"late\" }} }}");
            let form = format!(
                "update={}&actor=tester&source={}",
                form_encode(&update),
                form_encode("seeds:test-fault")
            );
            r.send("/update", "application/x-www-form-urlencoded", &form, true)
                .expect("inject a fact");
        }));
    });
    let args = crate::native::cli::ArchiveArgs {
        closed_before: "2026-10-01T00:00:00Z".into(),
        protect: protect.clone(),
        batch: 100,
        limit: 0,
        execute: true,
        log: Some(log.clone()),
    };
    let result = crate::native::archive_cli::archive(false, &c, &r, &args);
    super::archive::BEFORE_UPDATE.with(|h| *h.borrow_mut() = None);
    let err = result.expect_err("verification must fail");
    assert!(
        err.message.contains("verification FAILED"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains(&a),
        "the error names the batch ids: {}",
        err.message
    );
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.contains("\"phase\":\"intent\"") && text.contains(&a),
        "the intent was logged before the update: {text}"
    );
    assert!(
        !text.contains("\"phase\":\"result\""),
        "a failed batch has no result line: {text}"
    );
    assert!(
        r.snapshot_items(&ids).unwrap().seeds.is_empty(),
        "the committed batch did leave the live board"
    );

    let un = crate::native::cli::UnarchiveArgs {
        ids: Vec::new(),
        from_log: Some(log.clone()),
        batch: 100,
        execute: true,
    };
    crate::native::archive_cli::unarchive(false, &c, &r, &un).unwrap();
    let mut after = r.family_facts_for_test(&live, &ids);
    for facts in after.values_mut() {
        facts.retain(|(p, _)| p != injected);
    }
    assert_eq!(before, after, "the log restores every original fact");
    assert!(
        r.family_facts_for_test(&vocab::archive_graph(&live), &ids)
            .is_empty(),
        "nothing left in the archive"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
