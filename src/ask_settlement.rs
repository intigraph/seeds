//! A Seeds-owned, atomic Ask settlement. No runtime verifier or writer is selected.
//!
//! A native caller must supply an admitted receipt reader. Self-asserted JSON
//! (including a `verified` boolean) is not a reader and grants no authority.
//! This API does not enroll a signer, grant graph access, or retry a write.

use crate::backend::{Backend, Ctx, SeedWrite, WriteBatch};
use crate::error::{Result, SdError};
use crate::model::{canonical_time, Comment, Obj, Seed, Snapshot};
use crate::vocab;

/// Revision-bound operation selected by the caller, not by an answer payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Exact admitted Seeds project graph; never an implicit/default dataset.
    pub graph: String,
    /// The open Ask.
    pub ask: String,
    /// Its presented revision.
    pub ask_revision: u64,
    /// The work item or epic named by the Ask's `unblocks` edge.
    pub target: String,
    /// The linked item's presented revision.
    pub target_revision: u64,
    /// Exact native attestation receipt IRI; also the idempotency identity.
    pub receipt: String,
}

/// The trusted native reader's result. It is deliberately not deserializable.
///
/// Implementations of [`AdmittedSettlement`] must verify the entire signed
/// proposal, identity, graph scope, declared choice and deadline, including
/// BOTH item/revision bindings. A receipt for the older single-item contract
/// does not establish the linked item's binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// Full signed operation identity and item/revision bindings.
    pub binding: Request,
    /// The declared choice read from the signed proposal.
    pub choice: String,
    /// Server-verified human identity, never the settlement worker's claim.
    pub actor: String,
    /// Server receipt timestamp (RFC 3339).
    pub recorded_at: String,
}

/// Port owned by the native admission layer, never by browser input.
///
/// No production implementation is selected here. An unavailable, partial,
/// foreign or unverified response must return an error before any write.
pub trait AdmittedSettlement {
    /// Durably reserve the ONLY send for this exact intent before returning.
    /// Refuse a second reservation, including after an uncertain commit.
    /// The native adapter must fsync custody; there is no reset/release here.
    fn reserve_send(&self, request: &Request) -> Result<()>;

    /// Resolve and verify this exact native receipt under the admitted profile.
    fn read(&self, receipt: &str) -> Result<Answer>;
}

/// Persisted result, returned only after both items and the ruling read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    /// Closed Ask's revision (which can advance later during reconciliation).
    pub ask_revision: u64,
    /// Linked item's revision (which can advance after the settlement).
    pub target_revision: u64,
    /// Exact linked-item comment position.
    pub comment_index: u64,
    /// Exact attestation receipt.
    pub receipt: String,
    /// Commit receipt when this backend supplies one; absent on reconciliation.
    pub tx: Option<u64>,
    /// This call performed read-only reconciliation of an existing settlement.
    pub already_settled: bool,
}

fn receipt_predicate() -> String {
    vocab::seeds("askSettlementReceipt")
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn checked_request(req: &Request) -> Result<()> {
    let valid_id = |id: &str| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    };
    let nonce = req.receipt.strip_prefix("urn:quipu:attest:");
    if req.graph.is_empty()
        || req.graph.len() > 2048
        || req.graph.chars().any(char::is_control)
        || !valid_id(&req.ask)
        || !valid_id(&req.target)
        || req.ask == req.target
        || req.ask_revision == 0
        || req.target_revision == 0
        || nonce.is_none_or(|n| {
            n.len() != 64
                || !n
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
    {
        return Err(SdError::refused("invalid Ask settlement binding"));
    }
    Ok(())
}

fn ruling(answer: &Answer) -> Result<String> {
    if !valid_text(&answer.choice, 256)
        || !valid_text(&answer.actor, 256)
        || canonical_time(&answer.recorded_at, false).is_none()
    {
        return Err(SdError::refused("incomplete admitted Ask answer"));
    }
    Ok(format!(
        "Operator ruling: {}\nVerified actor: {}\nRecorded at: {}\nAnswer receipt: {}",
        answer.choice, answer.actor, answer.recorded_at, answer.binding.receipt
    ))
}

fn bound_items<'a>(snap: &'a Snapshot, req: &Request) -> Result<(&'a Seed, &'a Seed)> {
    let ask = snap.get(&req.ask)?;
    let target = snap.get(&req.target)?;
    if ask.issue_type != "ask"
        || ask.unblocks.as_deref() != Some(req.target.as_str())
        || ask.ephemeral != target.ephemeral
        || target.is_tombstone()
    {
        return Err(SdError::refused(
            "Ask type, linked item or graph binding changed",
        ));
    }
    Ok((ask, target))
}

fn observed(
    snap: &Snapshot,
    req: &Request,
    body: &str,
    tx: Option<u64>,
    already_settled: bool,
) -> Result<Settlement> {
    let (ask, target) = bound_items(snap, req)?;
    let marker = (receipt_predicate(), Obj::Iri(req.receipt.clone()));
    let ask_revision = req
        .ask_revision
        .checked_add(1)
        .ok_or_else(|| SdError::refused("Ask revision exhausted"))?;
    let target_revision = req
        .target_revision
        .checked_add(1)
        .ok_or_else(|| SdError::refused("target revision exhausted"))?;
    if ask
        .extra
        .iter()
        .filter(|(p, _)| p == &receipt_predicate())
        .count()
        != 1
    {
        return Err(SdError::failed(
            "Ask settlement receipt identity is absent or contradictory",
        ));
    }
    let comments = snap.comments_on(&req.target);
    let found: Vec<_> = comments
        .iter()
        .filter(|comment| comment.extra.contains(&marker))
        .collect();
    if ask.status != "closed"
        || ask.close_reason.as_deref() != Some(body)
        || !ask.extra.contains(&marker)
        || ask.revision < ask_revision
        || target.revision < target_revision
        || found.len() != 1
        || found[0].text != body
        || found[0]
            .extra
            .iter()
            .filter(|(p, _)| p == &receipt_predicate())
            .count()
            != 1
    {
        return Err(SdError::failed("Ask settlement read-back is unproven; retain the receipt and reconcile, never auto-resend"));
    }
    Ok(Settlement {
        ask_revision: ask.revision,
        target_revision: target.revision,
        comment_index: found[0].index,
        receipt: req.receipt.clone(),
        tx,
        already_settled,
    })
}

/// Close one open Ask and append its ruling to linked work in ONE guarded batch.
///
/// `reader: None` is disabled, and refuses before touching storage. There is no
/// sequential fallback. Every error, including a post-commit read failure, is
/// returned without an automatic resend. A later call with the SAME verified
/// receipt can reconcile an already committed result without writing again.
pub fn settle(
    b: &mut dyn Backend,
    ctx: &Ctx,
    req: &Request,
    reader: Option<&dyn AdmittedSettlement>,
) -> Result<Settlement> {
    let reader = reader.ok_or_else(|| {
        SdError::refused("Ask settlement has no admitted native answer reader; disabled")
    })?;
    checked_request(req)?;
    if b.project_graph() != Some(req.graph.as_str()) {
        return Err(SdError::refused(
            "Ask settlement project graph is foreign or unproven",
        ));
    }
    let answer = reader.read(&req.receipt)?;
    if answer.binding != *req {
        return Err(SdError::refused(
            "verified answer is bound to another Ask, target, revision or receipt",
        ));
    }
    let body = ruling(&answer)?;
    let ids = [req.ask.clone(), req.target.clone()];
    let snap = b.snapshot_items(&ids)?;
    let (ask, target) = bound_items(&snap, req)?;
    let marker = (receipt_predicate(), Obj::Iri(req.receipt.clone()));
    if ask.status == "closed" || ask.extra.contains(&marker) {
        return observed(&snap, req, &body, None, true);
    }
    if ask.status != "open"
        || target.status == "closed"
        || ask.revision != req.ask_revision
        || target.revision != req.target_revision
    {
        return Err(SdError::conflict(
            "Ask or linked item is stale, closed or not open for settlement",
        ));
    }
    if ask.extra.iter().any(|(p, _)| p == &receipt_predicate()) {
        return Err(SdError::refused(
            "Ask already carries another settlement identity",
        ));
    }
    let mut closed = ask.clone();
    closed.status = "closed".into();
    closed.closed_at = Some(ctx.now.clone());
    closed.close_reason = Some(body.clone());
    closed.outcome = Some("done".into());
    closed.updated_at = ctx.now.clone();
    closed.revision = closed
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::refused("Ask revision exhausted"))?;
    closed.extra.insert(marker.clone());
    let mut linked = target.clone();
    linked.updated_at = ctx.now.clone();
    linked.revision = linked
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::refused("target revision exhausted"))?;
    let index = snap
        .comments_on(&req.target)
        .iter()
        .map(|c| c.index)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| SdError::refused("comment positions exhausted"))?;
    let comment = Comment {
        seed: req.target.clone(),
        index,
        author: ctx.actor.clone(),
        text: body.clone(),
        created_at: ctx.now.clone(),
        extra: [marker.clone()].into(),
    };
    if snap
        .comments_on(&req.target)
        .iter()
        .any(|c| c.extra.contains(&marker))
    {
        return Err(SdError::refused(
            "receipt already appears on the linked item without a complete Ask settlement",
        ));
    }
    reader.reserve_send(req)?;
    let tx = b.commit(
        &WriteBatch {
            seeds: vec![
                SeedWrite {
                    seed: closed,
                    expected_revision: Some(req.ask_revision),
                },
                SeedWrite {
                    seed: linked,
                    expected_revision: Some(req.target_revision),
                },
            ],
            comments: vec![comment],
            source: "seeds:ask-settlement".into(),
            ..WriteBatch::default()
        },
        ctx,
    )?;
    observed(
        &b.snapshot_items(&ids)?,
        req,
        &body,
        (tx != 0).then_some(tx),
        false,
    )
}

/// Read-only recovery after any failed/uncertain attempt. It never reserves or
/// sends a write, even when the receipt is not yet observable in the board.
pub fn reconcile(
    b: &dyn Backend,
    req: &Request,
    reader: Option<&dyn AdmittedSettlement>,
) -> Result<Settlement> {
    let reader = reader.ok_or_else(|| {
        SdError::refused("Ask settlement has no admitted native answer reader; disabled")
    })?;
    checked_request(req)?;
    if b.project_graph() != Some(req.graph.as_str()) {
        return Err(SdError::refused(
            "Ask settlement project graph is foreign or unproven",
        ));
    }
    let answer = reader.read(&req.receipt)?;
    if answer.binding != *req {
        return Err(SdError::refused("verified answer binding changed"));
    }
    let body = ruling(&answer)?;
    observed(
        &b.snapshot_items(&[req.ask.clone(), req.target.clone()])?,
        req,
        &body,
        None,
        true,
    )
}
