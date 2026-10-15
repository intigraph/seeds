//! Revision-checked specification replacement and owner acknowledgement.
//! Metadata is ordinary, queryable graph facts; history remains bitemporal.

use crate::backend::{Backend, Ctx, SeedWrite, WriteBatch};
use crate::error::{Result, SdError};
use crate::model::{Comment, Obj, Seed};
use crate::vocab;

/// Configurable byte cap, including the generated version heading.
pub const DEFAULT_CAP: usize = 4096;

/// Portable definition for Quipu's validated stored-query registry.
pub const AWARENESS_DEFINITION: &str = include_str!("../queries/spec-awareness.json");

/// Single-valued metadata this build owns, while preserving unknown future facts.
pub const PREDICATES: &[&str] = &[
    "specVersion",
    "specPreviousVersion",
    "specChangedAt",
    "specChangedBy",
    "specChangedSession",
    "specRequester",
    "specSummary",
    "specUrgent",
    "specAckVersion",
    "specAckActor",
    "specAckAt",
    "specNotificationReceipt",
    "specStartVersion",
    "specStartActor",
    "specStartSession",
    "specStartAt",
    "specAckSession",
];

/// Persistent delivery intent consumed by the shared scheduled notifier.
#[derive(Debug, serde::Serialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: u8,
    pub labels: Vec<String>,
    pub episode: String,
    pub recipient: Option<String>,
    pub version: u64,
    pub previous_version: u64,
    pub summary: String,
    pub changed_at: String,
    pub urgent: bool,
    pub receipt: Option<String>,
}

fn text(seed: &Seed, name: &str) -> Result<Option<String>> {
    match one(seed, name)? {
        None => Ok(None),
        Some(Obj::Str(value)) => Ok(Some(value.clone())),
        _ => Err(SdError::failed(format!("invalid {name}"))),
    }
}

/// Read the exact generation even after it stops being pending.
pub fn receipt(b: &dyn Backend, id: &str, expected: u64) -> Result<Event> {
    let snapshot = b.snapshot_items(&[id.to_string()])?;
    event(snapshot.get(id)?, expected)
}

fn event(seed: &Seed, expected: u64) -> Result<Event> {
    if expected == 0 || number(seed, "specVersion")? != Some(expected) {
        return Err(SdError::usage("spec delivery generation changed"));
    }
    let previous = number(seed, "specPreviousVersion")?
        .ok_or_else(|| SdError::failed("missing prior spec version"))?;
    let urgent = match number(seed, "specUrgent")? {
        Some(0) => false,
        Some(1) => seed.priority == 0,
        _ => return Err(SdError::failed("spec urgency UNKNOWN")),
    };
    if previous >= expected {
        return Err(SdError::failed("invalid prior spec generation"));
    }
    Ok(Event {
        id: seed.id.clone(),
        title: seed.title.clone(),
        status: seed.status.clone(),
        priority: seed.priority,
        labels: seed.labels.iter().cloned().collect(),
        episode: expected.to_string(),
        recipient: seed.assignee.clone(),
        version: expected,
        previous_version: previous,
        summary: text(seed, "specSummary")?
            .ok_or_else(|| SdError::failed("missing spec summary"))?,
        changed_at: text(seed, "specChangedAt")?
            .ok_or_else(|| SdError::failed("missing spec timestamp"))?,
        urgent,
        receipt: text(seed, "specNotificationReceipt")?,
    })
}

/// Retained pending intents; polls never mint or increment a generation.
pub fn events(b: &dyn Backend) -> Result<Vec<Event>> {
    changes_read(b, true)
}

/// All unacknowledged changes, including already delivered pointers.
pub fn changes(b: &dyn Backend) -> Result<Vec<Event>> {
    changes_read(b, false)
}

fn changes_read(b: &dyn Backend, pending_only: bool) -> Result<Vec<Event>> {
    let ids = b.spec_change_ids()?;
    if ids.len() > 500 {
        return Err(SdError::failed("spec change ceiling reached"));
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let snapshot = b.snapshot_items(&ids)?;
    let mut out = Vec::new();
    for id in ids {
        let seed = snapshot.get(&id)?;
        let current = version(seed)?;
        let value = event(seed, current)?;
        let inbox = seed
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("inbox"))
            || seed.title.to_ascii_lowercase().starts_with("inbox:")
            || seed.title.to_ascii_lowercase().starts_with("mail:");
        if !inbox
            && unacked(seed)?
            && (!pending_only || value.receipt.is_none())
            && !matches!(seed.status.as_str(), "closed" | "tombstone")
        {
            out.push(value);
        }
    }
    Ok(out)
}

/// Confirm delivery separately from owner acknowledgement. Unknown outcomes
/// are resolved by `receipt`, never by sending another owner notification.
pub fn receipt_ack(
    b: &mut dyn Backend,
    ctx: &Ctx,
    id: &str,
    expected: u64,
    delivered: &str,
) -> Result<Option<u64>> {
    if delivered.trim().is_empty() || delivered.len() > 128 {
        return Err(SdError::usage(
            "notification receipt must be a bounded opaque ID",
        ));
    }
    let snapshot = b.snapshot_items(&[id.to_string()])?;
    let before = snapshot.get(id)?;
    let value = event(before, expected)?;
    if let Some(existing) = value.receipt {
        return if existing == delivered {
            Ok(None)
        } else {
            Err(SdError::usage("spec delivery receipt conflict"))
        };
    }
    let mut seed = before.clone();
    set(
        &mut seed,
        "specNotificationReceipt",
        Obj::Str(delivered.to_string()),
    );
    seed.updated_at = ctx.now.clone();
    seed.revision = seed
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::failed("item revision overflow"))?;
    let tx = b.commit(
        &WriteBatch {
            seeds: vec![SeedWrite {
                expected_revision: Some(before.revision),
                seed,
            }],
            source: "seeds:spec-delivery-ack".into(),
            ..Default::default()
        },
        ctx,
    )?;
    Ok(Some(tx))
}

fn one<'a>(seed: &'a Seed, name: &str) -> Result<Option<&'a Obj>> {
    let predicate = vocab::seeds(name);
    let values: Vec<_> = seed.extra.iter().filter(|(p, _)| p == &predicate).collect();
    match values.as_slice() {
        [] => Ok(None),
        [(_, value)] => Ok(Some(value)),
        _ => Err(SdError::failed(format!("{}: conflicting {name}", seed.id))),
    }
}

fn number(seed: &Seed, name: &str) -> Result<Option<u64>> {
    match one(seed, name)? {
        None => Ok(None),
        Some(Obj::Int(n)) if *n >= 0 => Ok(Some(*n as u64)),
        _ => Err(SdError::failed(format!("{}: invalid {name}", seed.id))),
    }
}

fn set(seed: &mut Seed, name: &str, value: Obj) {
    let predicate = vocab::seeds(name);
    seed.extra.retain(|(p, _)| p != &predicate);
    seed.extra.insert((predicate, value));
}

fn heading(text: &str) -> Option<u64> {
    text.lines()
        .next()?
        .trim()
        .strip_prefix("SPEC v")?
        .parse()
        .ok()
}

/// The governed spec generation, independent of ordinary item edits.
pub fn version(seed: &Seed) -> Result<u64> {
    match number(seed, "specVersion")? {
        Some(0) => Err(SdError::failed("governed spec generation cannot be zero")),
        Some(value) => Ok(value),
        None => Ok(seed.description.as_deref().and_then(heading).unwrap_or(0)),
    }
}

/// Whether this item has a governed generation, rather than just a legacy heading.
pub fn governed(seed: &Seed) -> Result<bool> {
    Ok(number(seed, "specVersion")?.is_some())
}

/// Whether the current assignee has acknowledged the current generation.
pub fn unacked(seed: &Seed) -> Result<bool> {
    let current = number(seed, "specVersion")?;
    let actor = match one(seed, "specAckActor")? {
        None => None,
        Some(Obj::Iri(value)) => Some(value),
        _ => return Err(SdError::failed("invalid spec acknowledgement actor")),
    };
    let assigned = seed.assignee.as_deref().map(vocab::principal_iri);
    let acknowledged = number(seed, "specAckVersion")?;
    Ok(current.is_some_and(|n| acknowledged != Some(n) || actor != assigned.as_ref()))
}

/// All inputs needed for one atomic replacement; clocks and identity are injected.
pub struct RescopeReq<'a> {
    pub spec: &'a str,
    pub quote: &'a str,
    pub requester: &'a str,
    pub summary: &'a str,
    pub urgent: bool,
    pub dry_run: bool,
    pub byte_cap: usize,
}

/// Replace the description and append the quote/provenance in ONE transaction.
pub fn rescope(
    b: &mut dyn Backend,
    ctx: &Ctx,
    id: &str,
    req: &RescopeReq<'_>,
) -> Result<(Seed, u64)> {
    let session = ctx
        .claims
        .session
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| SdError::usage("rescope requires --session provenance"))?;
    if req.byte_cap == 0 || req.byte_cap > 65536 {
        return Err(SdError::usage("spec byte cap must be 1..65536"));
    }
    if req.spec.trim().is_empty() || req.spec.len() > req.byte_cap {
        return Err(SdError::usage(
            "spec is empty or exceeds configured byte cap",
        ));
    }
    if req.requester.trim().is_empty()
        || (req.requester != ctx.actor && req.quote.trim().is_empty())
    {
        return Err(SdError::usage(
            "requester and a verbatim quote are required for another requester",
        ));
    }
    if req.quote.len() > req.byte_cap || req.summary.len() > 160 || req.summary.trim().is_empty() {
        return Err(SdError::usage(
            "quote exceeds cap or summary is empty/exceeds 160 bytes",
        ));
    }
    let lower = req
        .spec
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if ["supersedes the above", "scope expansion"]
        .iter()
        .any(|marker| lower.contains(marker))
        || lower
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|word| word == "addition")
    {
        return Err(SdError::usage(
            "layering marker refused; provide one consolidated spec",
        ));
    }
    let snapshot = b.snapshot_items(&[id.to_string()])?;
    let before = snapshot.get(id)?;
    if req.urgent && before.priority != 0 {
        return Err(SdError::usage(
            "urgent specification delivery is reserved for P0",
        ));
    }
    let previous = version(before)?;
    let next = previous
        .checked_add(1)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| SdError::failed("spec version overflow"))?;
    let body = if heading(req.spec).is_some() {
        req.spec.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        req.spec
    };
    if body.trim().is_empty() {
        return Err(SdError::usage("spec body is empty"));
    }
    let description = format!("SPEC v{next}\n{}", body.trim());
    if description.len() > req.byte_cap {
        return Err(SdError::usage("stamped spec exceeds configured byte cap"));
    }
    let mut seed = before.clone();
    seed.description = Some(description);
    seed.updated_at = ctx.now.clone();
    seed.revision = seed
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::failed("item revision overflow"))?;
    for (name, value) in [
        ("specVersion", Obj::Int(next as i64)),
        ("specPreviousVersion", Obj::Int(previous as i64)),
        ("specChangedAt", Obj::Str(ctx.now.clone())),
        ("specChangedBy", Obj::Iri(vocab::principal_iri(&ctx.actor))),
        ("specChangedSession", Obj::Str(session.to_string())),
        (
            "specRequester",
            Obj::Iri(vocab::principal_iri(req.requester)),
        ),
        ("specSummary", Obj::Str(req.summary.to_string())),
        ("specUrgent", Obj::Int(i64::from(req.urgent))),
    ] {
        set(&mut seed, name, value);
    }
    // Old notification delivery must never stand for delivery of this generation.
    seed.extra
        .retain(|(p, _)| p != &vocab::seeds("specNotificationReceipt"));
    let index = snapshot
        .comments_on(id)
        .iter()
        .map(|c| c.index)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| SdError::failed("comment index overflow"))?;
    let comment = Comment {
        seed: id.to_string(), index, author: ctx.actor.clone(), created_at: ctx.now.clone(),
        text: format!("SPEC v{previous} -> v{next}\nRequester: {}\nActor: {}\nSession: {session}\nAt: {}\nSummary: {}\nVerbatim quote:\n{}",
            req.requester, ctx.actor, ctx.now, req.summary, req.quote),
        extra: Default::default(),
    };
    if req.dry_run {
        return Ok((seed, 0));
    }
    let tx = b.commit(
        &WriteBatch {
            seeds: vec![SeedWrite {
                expected_revision: Some(before.revision),
                seed: seed.clone(),
            }],
            comments: vec![comment],
            source: "seeds:rescope".into(),
            ..Default::default()
        },
        ctx,
    )?;
    Ok((seed, tx))
}

/// Acknowledge only the exact generation observed by its current assignee.
/// Repeated acknowledgement is a read-only success, not another write.
pub fn ack(b: &mut dyn Backend, ctx: &Ctx, id: &str, expected: u64) -> Result<(Seed, Option<u64>)> {
    let snapshot = b.snapshot_items(&[id.to_string()])?;
    let before = snapshot.get(id)?;
    if before.assignee.as_deref() != Some(&ctx.actor) {
        return Err(SdError::usage(
            "only the current assignee can acknowledge the spec",
        ));
    }
    if expected == 0 || number(before, "specVersion")? != Some(expected) {
        return Err(SdError::usage(
            "spec generation changed or has no governed rescope; reread before ack",
        ));
    }
    if !unacked(before)? {
        return Ok((before.clone(), None));
    }
    let mut seed = before.clone();
    set(&mut seed, "specAckVersion", Obj::Int(expected as i64));
    set(
        &mut seed,
        "specAckActor",
        Obj::Iri(vocab::principal_iri(&ctx.actor)),
    );
    set(&mut seed, "specAckAt", Obj::Str(ctx.now.clone()));
    seed.extra
        .retain(|(p, _)| p != &vocab::seeds("specAckSession"));
    if let Some(session) = &ctx.claims.session {
        set(&mut seed, "specAckSession", Obj::Str(session.clone()));
    }
    seed.updated_at = ctx.now.clone();
    seed.revision = seed
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::failed("item revision overflow"))?;
    let tx = b.commit(
        &WriteBatch {
            seeds: vec![SeedWrite {
                expected_revision: Some(before.revision),
                seed: seed.clone(),
            }],
            source: "seeds:spec-ack".into(),
            ..Default::default()
        },
        ctx,
    )?;
    Ok((seed, Some(tx)))
}

/// Record the spec at an actual owner start/claim transition, once per session.
/// A leader changing somebody else's status does not invent an owner start.
pub fn mark_start(seed: &mut Seed, ctx: &Ctx) -> Result<()> {
    let Some(session) = ctx
        .claims
        .session
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(());
    };
    if seed.assignee.as_deref() != Some(&ctx.actor) || number(seed, "specVersion")?.is_none() {
        return Ok(());
    }
    if text(seed, "specStartSession")?.as_deref() == Some(session)
        && one(seed, "specStartActor")? == Some(&Obj::Iri(vocab::principal_iri(&ctx.actor)))
    {
        return Ok(());
    }
    let generation = version(seed)?;
    set(seed, "specStartVersion", Obj::Int(generation as i64));
    set(
        seed,
        "specStartActor",
        Obj::Iri(vocab::principal_iri(&ctx.actor)),
    );
    set(seed, "specStartSession", Obj::Str(session.to_string()));
    set(seed, "specStartAt", Obj::Str(ctx.now.clone()));
    Ok(())
}
