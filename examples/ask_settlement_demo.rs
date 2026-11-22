//! Local atomic-settlement demonstration. The receipt port is synthetic, not cryptography.
#![allow(clippy::unwrap_used)]
use seeds::ask_settlement::{self as settlement, AdmittedSettlement, Answer, Request};
use seeds::backend::{Backend, Ctx};
use seeds::engine::{self, CreateReq};
use seeds::error::{Result, SdError};
use seeds::quipu_backend::QuipuBackend;
use std::cell::Cell;

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
fn main() -> Result<()> {
    let (mut b, req, port) = fixture();
    let before = b.snapshot(None)?;
    println!("Local in-memory store; synthetic native admission port (not crypto)");
    println!(
        "Before: Ask={}, linked work={}, linked comments={}",
        before.get(&req.ask)?.status,
        before.get(&req.target)?.status,
        before.comments_on(&req.target).len()
    );
    let disabled = settlement::settle(&mut b, &ctx(), &req, None);
    println!("Without admitted port: {}", disabled.unwrap_err());
    assert_eq!(b.snapshot(None)?.tx, before.tx);
    let done = settlement::settle(&mut b, &ctx(), &req, Some(&port))?;
    let after = b.snapshot(None)?;
    println!("After one atomic batch: Ask={} revision={}, linked work={} revision={}, linked comments={}", after.get(&req.ask)?.status, done.ask_revision, after.get(&req.target)?.status, done.target_revision, after.comments_on(&req.target).len());
    println!("{}", after.comments_on(&req.target)[0].text);
    let recovered = settlement::reconcile(&b, &req, Some(&port))?;
    assert!(recovered.already_settled);
    assert_eq!(b.snapshot(None)?.tx, after.tx);
    println!("Read-only recovery: same receipt, still one ruling, transaction unchanged");
    Ok(())
}
