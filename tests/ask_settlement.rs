//! Synthetic admitted-port fixtures exercise real Seeds transactions, not crypto.
#![allow(clippy::unwrap_used)]
use seeds::ask_settlement::{self as settlement, AdmittedSettlement, Answer, Request};
use seeds::backend::{Backend, Ctx, WriteBatch};
use seeds::engine::{self, CreateReq, UpdateReq};
use seeds::error::{Result, SdError};
use seeds::model::Snapshot;
use seeds::quipu_backend::QuipuBackend;
use std::cell::Cell;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

struct Port {
    answer: Answer,
    reserved: Cell<bool>,
}
impl AdmittedSettlement for Port {
    fn read(&self, receipt: &str) -> Result<Answer> {
        if receipt != self.answer.binding.receipt {
            return Err(SdError::refused("foreign receipt"));
        }
        Ok(self.answer.clone())
    }
    fn reserve_send(&self, req: &Request) -> Result<()> {
        if *req != self.answer.binding || self.reserved.replace(true) {
            return Err(SdError::refused("send already reserved; reconcile only"));
        }
        Ok(())
    }
}
fn ctx() -> Ctx {
    Ctx {
        now: "2026-11-22T12:00:00Z".into(),
        actor: "settlement-fixture".into(),
        prefix: "test".into(),
        claims: Default::default(),
    }
}
fn fixture() -> (QuipuBackend, Request, Port) {
    let mut b = QuipuBackend::in_memory("urn:ask-settlement-fixture").unwrap();
    let target = engine::create(
        &mut b,
        &ctx(),
        &CreateReq {
            title: "Implement the choice".into(),
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0;
    let ask = engine::create(
        &mut b,
        &ctx(),
        &CreateReq {
            title: "Which color?".into(),
            issue_type: Some("ask".into()),
            unblocks: Some(target.id.clone()),
            default: Some("Keep blue".into()),
            expires_at: Some("2026-12-01T00:00:00Z".into()),
            ..CreateReq::default()
        },
    )
    .unwrap()
    .0;
    let req = Request {
        graph: "urn:ask-settlement-fixture".into(),
        ask: ask.id,
        ask_revision: ask.revision,
        target: target.id,
        target_revision: target.revision,
        receipt: format!("urn:quipu:attest:{}", "a".repeat(64)),
    };
    let port = Port {
        answer: Answer {
            binding: req.clone(),
            choice: "Blue".into(),
            actor: "fixture-operator".into(),
            recorded_at: "2026-11-22T11:59:00Z".into(),
        },
        reserved: Cell::new(false),
    };
    (b, req, port)
}
#[test]
fn atomic_settlement_closes_only_the_ask_and_records_the_linked_ruling_once() {
    let (mut b, req, port) = fixture();
    let result = settlement::settle(&mut b, &ctx(), &req, Some(&port)).unwrap();
    assert!(!result.already_settled);
    assert_eq!(
        (
            result.ask_revision,
            result.target_revision,
            result.comment_index
        ),
        (2, 2, 1)
    );
    let snap = b.snapshot(None).unwrap();
    assert_eq!(snap.get(&req.ask).unwrap().status, "closed");
    assert_eq!(snap.get(&req.target).unwrap().status, "open");
    let comments = snap.comments_on(&req.target);
    assert_eq!(comments.len(), 1);
    assert!(comments[0]
        .text
        .contains("Verified actor: fixture-operator"));
    assert!(comments[0].text.contains(&req.receipt));
    let tx = snap.tx;
    assert!(
        settlement::reconcile(&b, &req, Some(&port))
            .unwrap()
            .already_settled
    );
    assert!(
        settlement::settle(&mut b, &ctx(), &req, Some(&port))
            .unwrap()
            .already_settled
    );
    assert_eq!(
        b.snapshot(None).unwrap().tx,
        tx,
        "recovery never writes another ruling"
    );
}
#[test]
fn disabled_foreign_stale_and_unverified_operations_write_nothing() {
    for mode in 0..8 {
        let (mut b, mut req, mut port) = fixture();
        match mode {
            1 => req.ask_revision += 1,
            2 => req.target_revision += 1,
            3 => port.answer.binding.receipt = format!("urn:quipu:attest:{}", "b".repeat(64)),
            4 => port.answer.binding.target = "test-foreign".into(),
            5 => port.answer.actor.clear(),
            6 => port.answer.recorded_at = "not a timestamp".into(),
            7 => {
                req.graph = "urn:foreign-board".into();
                port.answer.binding = req.clone();
            }
            _ => {}
        }
        let before = b.snapshot(None).unwrap();
        let result = settlement::settle(
            &mut b,
            &ctx(),
            &req,
            if mode == 0 { None } else { Some(&port) },
        );
        assert!(result.is_err());
        let after = b.snapshot(None).unwrap();
        assert_eq!(after.seeds, before.seeds);
        assert_eq!(after.comments, before.comments);
        assert_eq!(after.tx, before.tx);
        assert!(!port.reserved.get());
    }
}
// Change one revision AFTER the read, before the actual atomic commit.
struct Race {
    inner: QuipuBackend,
    target: String,
    lose_reply: bool,
    sends: usize,
}
impl Backend for Race {
    fn project_graph(&self) -> Option<&str> {
        self.inner.project_graph()
    }
    fn snapshot(&self, at: Option<u64>) -> Result<Snapshot> {
        self.inner.snapshot(at)
    }
    fn snapshot_items(&self, ids: &[String]) -> Result<Snapshot> {
        self.inner.snapshot_items(ids)
    }
    fn ready_ids(&self, at: Option<u64>) -> Result<Vec<String>> {
        self.inner.ready_ids(at)
    }
    fn claims_of(&self, id: &str) -> Result<Vec<(u64, seeds::backend::Claims)>> {
        self.inner.claims_of(id)
    }
    fn commit(&mut self, batch: &WriteBatch, ctx: &Ctx) -> Result<u64> {
        self.sends += 1;
        if self.lose_reply {
            self.inner.commit(batch, ctx)?;
            return Err(SdError::failed(
                "synthetic lost reply after committed batch",
            ));
        }
        engine::update(
            &mut self.inner,
            ctx,
            std::slice::from_ref(&self.target),
            &UpdateReq {
                notes: Some("Concurrent edit".into()),
                ..UpdateReq::default()
            },
        )?;
        self.inner.commit(batch, ctx)
    }
}
#[test]
fn target_race_refuses_the_whole_batch_and_reserves_no_second_send() {
    let (b, req, port) = fixture();
    let mut raced = Race {
        inner: b,
        target: req.target.clone(),
        lose_reply: false,
        sends: 0,
    };
    assert!(settlement::settle(&mut raced, &ctx(), &req, Some(&port)).is_err());
    let snap = raced.snapshot(None).unwrap();
    assert_eq!(snap.get(&req.ask).unwrap().status, "open");
    assert!(snap.comments_on(&req.target).is_empty());
    assert!(settlement::reconcile(&raced, &req, Some(&port)).is_err());
    assert_eq!(raced.sends, 1);
    assert!(port.reserved.get());
}
#[test]
fn lost_reply_recovers_read_only_without_resending_the_atomic_close() {
    let (b, req, port) = fixture();
    let mut raced = Race {
        inner: b,
        target: req.target.clone(),
        lose_reply: true,
        sends: 0,
    };
    assert!(settlement::settle(&mut raced, &ctx(), &req, Some(&port)).is_err());
    let result = settlement::reconcile(&raced, &req, Some(&port)).unwrap();
    assert!(result.already_settled);
    assert_eq!(raced.sends, 1);
    assert_eq!(
        raced.snapshot(None).unwrap().comments_on(&req.target).len(),
        1
    );
}

#[test]
fn ask_race_also_refuses_the_linked_ruling_atomically() {
    let (b, req, port) = fixture();
    let mut raced = Race {
        inner: b,
        target: req.ask.clone(),
        lose_reply: false,
        sends: 0,
    };
    assert!(settlement::settle(&mut raced, &ctx(), &req, Some(&port)).is_err());
    let snap = raced.snapshot(None).unwrap();
    assert_eq!(snap.get(&req.ask).unwrap().status, "open");
    assert_eq!(snap.get(&req.target).unwrap().revision, req.target_revision);
    assert!(snap.comments_on(&req.target).is_empty());
}

#[test]
fn a_reserved_but_unobserved_send_stays_reconcile_only() {
    let (mut b, req, port) = fixture();
    port.reserved.set(true); // Native custody retained an earlier uncertain attempt.
    let before = b.snapshot(None).unwrap();
    assert!(settlement::reconcile(&b, &req, Some(&port)).is_err());
    assert!(settlement::settle(&mut b, &ctx(), &req, Some(&port)).is_err());
    let after = b.snapshot(None).unwrap();
    assert_eq!(after.tx, before.tx);
    assert_eq!(after.seeds, before.seeds);
    assert_eq!(after.comments, before.comments);
}
