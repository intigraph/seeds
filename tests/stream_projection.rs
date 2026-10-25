//! Actual shared producer -> shared applier -> Seeds reads, no live endpoint.
#![cfg(feature = "native")]
#![allow(clippy::unwrap_used)]
#![allow(clippy::disallowed_methods)]

use quipu::applier::Outcome;
use quipu::asof_share::{self, Limits, Request};
use quipu::replicate_source::{Frame, Session, StreamRequest};
use quipu::signing::SigningIdentity;
use ring::signature::Ed25519KeyPair;
use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq, ListReq, ReadyReq, UpdateReq};
use seeds::native::stream_projection::StreamBoard;
use seeds::projection::SourceBinding;
use seeds::quipu_backend::QuipuBackend;
use seeds::vocab;
use std::sync::Arc;

const GRAPH: &str = "urn:test:push-board";
fn ctx() -> Ctx {
    Ctx {
        now: "2026-10-25T00:00:00Z".into(),
        actor: "fixture".into(),
        prefix: "sd".into(),
        claims: Default::default(),
    }
}
struct Tree {
    source: QuipuBackend,
    replica: StreamBoard,
    session: Session,
    id: String,
}
impl Tree {
    fn new() -> Self {
        let mut source = QuipuBackend::in_memory(GRAPH).unwrap();
        source
            .store()
            .graph_create(&vocab::ephemeral_graph(GRAPH))
            .unwrap();
        let id = engine::create(
            &mut source,
            &ctx(),
            &CreateReq {
                title: "before".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .0
        .id;
        let spool = tempfile::tempdir().unwrap();
        let mut graphs = vec![GRAPH.into(), vocab::ephemeral_graph(GRAPH)];
        graphs.sort();
        let request = Request {
            graphs: graphs.clone(),
            context_graphs: vec![],
            tx: None,
            expected_store_id: source.store().store_id().unwrap(),
        };
        let cut =
            asof_share::cut(source.store(), &request, spool.path(), &Limits::default()).unwrap();
        let records: Vec<serde_json::Value> = std::fs::read_to_string(&cut.spool)
            .unwrap()
            .lines()
            .flat_map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                v["records"].as_array().unwrap().clone()
            })
            .collect();
        let binding = SourceBinding {
            store_id: cut.manifest.source.store_id.clone(),
            log_epoch: cut.manifest.source.log_epoch.clone(),
            scope_digest: cut.manifest.scope_digest.clone(),
            policy_digest: cut.manifest.policy_digest.clone(),
        };
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let public = quipu::signing::public_key_hex(&kp);
        let key = Arc::new(SigningIdentity::new(kp, "fixture"));
        let mut replica =
            StreamBoard::empty(GRAPH, cut.manifest.clone(), &binding, &public).unwrap();
        assert!(replica.list(&ListReq::default(), 0, 5000).is_err());
        let data = serde_json::json!({"manifest":cut.manifest,"records":records}).to_string();
        assert!(matches!(
            replica.apply_frame("quipu.asof", &data, 0).unwrap(),
            Outcome::Bootstrapped { .. }
        ));
        assert!(replica.list(&ListReq::default(), 0, 5000).is_err());
        let req = StreamRequest {
            graphs,
            context_graphs: vec![],
            since: cut.manifest.watermark_tx,
            expected_store_id: binding.store_id,
        };
        let (session, open) =
            Session::open(source.store(), &req, "fixture-session".into(), key).unwrap();
        assert!(matches!(
            replica.apply_frame(open.event, &open.data, 1).unwrap(),
            Outcome::SessionOpened { .. }
        ));
        Self {
            source,
            replica,
            session,
            id,
        }
    }
    fn feed(&mut self, frames: Vec<Frame>, now: u64) {
        for f in frames {
            self.replica.apply_frame(f.event, &f.data, now).unwrap();
        }
    }
    fn prove(&mut self, nonce: u8, sent: u64, received: u64) {
        self.replica.challenge([nonce; 16], sent).unwrap();
        let (frames, answered) = self
            .session
            .poll(self.source.store(), 100, Some([nonce; 16]))
            .unwrap();
        assert!(answered);
        self.feed(frames, received);
    }
}

#[test]
fn signed_changes_reach_board_plate_and_ready_without_remote_reads() {
    let mut t = Tree::new();
    let heartbeat = t.session.heartbeat();
    t.feed(vec![heartbeat], 2);
    assert!(
        t.replica.list(&ListReq::default(), 2, 5000).is_err(),
        "heartbeat is not proof"
    );
    t.prove(1, 3, 4);
    let first = t.replica.list(&ListReq::default(), 4, 5000).unwrap();
    assert_eq!(first.value.issues[0].title, "before");
    assert_eq!(first.bounded_freshness_ms, Some(1));
    engine::update(
        &mut t.source,
        &ctx(),
        std::slice::from_ref(&t.id),
        &UpdateReq {
            title: Some("after".into()),
            assignee: Some("reader".into()),
            ..Default::default()
        },
    )
    .unwrap();
    t.prove(2, 5, 6);
    let mut plate = ListReq::default();
    plate.filter.assignee = Some("reader".into());
    for _ in 0..20 {
        let got = t.replica.list(&plate, 6, 5000).unwrap();
        assert_eq!(got.value.issues[0].title, "after");
        assert_eq!(got.progress.source_tx, t.source.snapshot(None).unwrap().tx);
        assert_eq!(
            t.replica
                .ready(&ctx(), &ReadyReq::default(), 6, 5000)
                .unwrap()
                .value
                .issues[0]
                .id,
            t.id
        );
    }
}

#[test]
fn disconnect_expiry_clock_change_and_bad_signature_refuse_bounded_reads() {
    let mut t = Tree::new();
    t.prove(1, 10, 11);
    assert!(t.replica.list(&ListReq::default(), 5011, 5000).is_err());
    t.replica.disconnect();
    assert!(t.replica.stream_request().unwrap().since > 0);
    assert!(t.replica.list(&ListReq::default(), 12, 5000).is_err());
    assert!(t.replica.challenge([2; 16], 12).is_err());
    let mut t = Tree::new();
    t.prove(1, 10, 11);
    t.replica.clock_discontinuity();
    assert!(t.replica.list(&ListReq::default(), 12, 5000).is_err());
    let mut t = Tree::new();
    t.prove(1, 10, 11);
    let mut frame: serde_json::Value = serde_json::from_str(&t.session.heartbeat().data).unwrap();
    frame["signature"] = serde_json::json!("00".repeat(64));
    let _ = t
        .replica
        .apply_frame("quipu.replicate.coverage", &frame.to_string(), 12)
        .unwrap();
    assert!(t.replica.list(&ListReq::default(), 12, 5000).is_err());
    assert!(t.replica.apply_frame("quipu.changes", "{}", 13).is_err());
}

#[test]
fn a_partial_transaction_stays_invisible_until_the_shared_commit_barrier() {
    let mut t = Tree::new();
    t.prove(1, 1, 2);
    t.session = t.session.with_part_max_len(1024);
    engine::update(
        &mut t.source,
        &ctx(),
        std::slice::from_ref(&t.id),
        &UpdateReq {
            title: Some("committed after barrier".into()),
            assignee: Some("reader".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let (frames, _) = t.session.poll(t.source.store(), 100, None).unwrap();
    assert!(
        frames.iter().any(|f| f.event.ends_with("begin")),
        "{frames:?}"
    );
    let first = frames.first().unwrap();
    assert!(matches!(
        t.replica.apply_frame(first.event, &first.data, 3).unwrap(),
        Outcome::Buffered { .. }
    ));
    assert_eq!(
        t.replica
            .list(&ListReq::default(), 3, 5000)
            .unwrap()
            .value
            .issues[0]
            .title,
        "before"
    );
    t.feed(frames.into_iter().skip(1).collect(), 4);
    t.prove(2, 5, 6);
    assert_eq!(
        t.replica
            .list(&ListReq::default(), 6, 5000)
            .unwrap()
            .value
            .issues[0]
            .title,
        "committed after barrier"
    );
    assert!(
        t.replica.challenge([2; 16], 7).is_err(),
        "an issued nonce cannot be reused"
    );
}

#[test]
fn a_signed_epoch_end_stops_serving_the_old_board() {
    let mut t = Tree::new();
    t.prove(1, 1, 2);
    let packs = tempfile::tempdir().unwrap();
    t.source
        .store_mut()
        .freeze_graph(GRAPH, packs.path().to_str().unwrap(), &ctx().now, None)
        .unwrap();
    let (frames, _) = t.session.poll(t.source.store(), 100, None).unwrap();
    assert!(frames.iter().any(|f| f.event.ends_with("control")));
    t.feed(frames, 3);
    assert!(t.replica.list(&ListReq::default(), 3, 5000).is_err());
}
