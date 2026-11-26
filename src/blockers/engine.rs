//! Revision-checked blocker reconciliation and receipt acknowledgement.
use super::{
    condition, default_recheck, event_id, gate_lapsed, load, recheck_instant, save, Blocker, Gate,
    Ledger, State, UnlockEvent,
};
use crate::backend::{Backend, Ctx, SeedWrite, WriteBatch};
use crate::error::{Result, SdError};
use crate::model::Seed;

fn write(
    b: &mut dyn Backend,
    ctx: &Ctx,
    before: &Seed,
    mut after: Seed,
    ledger: &Ledger,
) -> Result<u64> {
    save(&mut after, ledger)?;
    after.updated_at = ctx.now.clone();
    after.revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| SdError::failed("revision overflow"))?;
    b.commit(
        &WriteBatch {
            seeds: vec![SeedWrite {
                seed: after,
                expected_revision: Some(before.revision),
            }],
            source: "seeds:blockers".into(),
            ..WriteBatch::default()
        },
        ctx,
    )
}

/// Declare an external prerequisite; an unresolved read holds an open item.
pub fn add(b: &mut dyn Backend, ctx: &Ctx, id: &str, blocker: Blocker) -> Result<u64> {
    let snap = b.snapshot_seeds(&[id.into()])?;
    let before = snap.get(id)?;
    if before.is_tombstone() || before.status == "closed" {
        return Err(SdError::refused(
            "closed or deleted seed cannot gain a blocker",
        ));
    }
    let mut ledger = load(before)?;
    if ledger.blockers.contains_key(&blocker.iri) {
        return Err(SdError::refused(
            "blocker already declared; remove it before changing its condition",
        ));
    }
    let mut blocker = blocker;
    blocker.state = condition::observe(b, &blocker, None);
    blocker.observed_at = Some(ctx.now.clone());
    ledger.blockers.insert(blocker.iri.clone(), blocker);
    let mut after = before.clone();
    if after.status == "open" && ledger.blockers.values().any(|x| x.state != State::Done) {
        after.status = "blocked".into();
        ledger.managed_status = true;
    }
    write(b, ctx, before, after, &ledger)
}

/// Block a seed on a human or decision gate: WHO must act, rechecked at
/// `recheck_at` (default: now plus [`super::DEFAULT_RECHECK`]). A gate with no
/// one named or nothing awaited is refused, as is a recheck instant already
/// past: such a block could never be checked. Lapsed gates the seed carried
/// are replaced, so re-blocking starts a new episode.
pub fn gate(
    b: &mut dyn Backend,
    ctx: &Ctx,
    id: &str,
    who: &str,
    reason: &str,
    recheck_at: Option<&str>,
) -> Result<u64> {
    let (who, reason) = (who.trim(), reason.trim());
    if who.is_empty() || reason.is_empty() {
        return Err(SdError::usage(
            "a gate names who must act (--who) and what is awaited (--reason)",
        ));
    }
    let recheck_at = match recheck_at {
        Some(v) => recheck_instant(v)?,
        None => default_recheck(&ctx.now)?,
    };
    if recheck_at.as_str() <= ctx.now.as_str() {
        return Err(SdError::usage(format!(
            "recheck_at {recheck_at} has already passed; a gate must lapse in the future"
        )));
    }
    let snap = b.snapshot_seeds(&[id.into()])?;
    let before = snap.get(id)?;
    if before.is_tombstone() || before.status == "closed" {
        return Err(SdError::refused(
            "closed or deleted seed cannot gain a gate",
        ));
    }
    let mut ledger = load(before)?;
    ledger.gates.retain(|g| !gate_lapsed(g, &ctx.now));
    ledger.gates.push(Gate {
        who: who.into(),
        reason: reason.into(),
        recheck_at,
        actor: ctx.actor.clone(),
        created_at: ctx.now.clone(),
        lapse_generation: None,
    });
    let mut after = before.clone();
    if after.status == "open" {
        after.status = "blocked".into();
        ledger.managed_status = true;
    }
    write(b, ctx, before, after, &ledger)
}

/// Remove every gate. A status this feature set returns to open when no
/// other prerequisite holds; one set by hand is left as it is.
pub fn ungate(b: &mut dyn Backend, ctx: &Ctx, id: &str) -> Result<Option<u64>> {
    let snap = b.snapshot_seeds(&[id.into()])?;
    let before = snap.get(id)?;
    let mut ledger = load(before)?;
    if ledger.gates.is_empty() {
        return Ok(None);
    }
    ledger.gates.clear();
    let mut after = before.clone();
    if ledger.managed_status
        && after.status == "blocked"
        && ledger.blockers.values().all(|x| x.state == State::Done)
    {
        after.status = "open".into();
        ledger.managed_status = false;
    }
    write(b, ctx, before, after, &ledger).map(Some)
}

/// Remove one external prerequisite.
pub fn remove(b: &mut dyn Backend, ctx: &Ctx, id: &str, iri: &str) -> Result<Option<u64>> {
    condition::validate_iri(iri)?;
    let snap = b.snapshot_seeds(&[id.into()])?;
    let before = snap.get(id)?;
    let mut ledger = load(before)?;
    if ledger.blockers.remove(iri).is_none() {
        return Ok(None);
    }
    write(b, ctx, before, before.clone(), &ledger).map(Some)
}

/// Reconcile one seed. No-op polls do not write or generate another event.
pub fn reconcile(b: &mut dyn Backend, ctx: &Ctx, id: &str) -> Result<Option<u64>> {
    let first = b.snapshot_seeds(&[id.into()])?;
    let before = first.get(id)?.clone();
    let mut ledger = load(&before)?;
    let original = ledger.clone();
    let mut after = before.clone();
    for blocker in ledger.blockers.values_mut() {
        let state = condition::observe(b, blocker, None);
        if state != blocker.state {
            blocker.state = state;
            blocker.observed_at = Some(ctx.now.clone());
        }
    }
    let external_done = ledger.blockers.values().all(|x| x.state == State::Done);
    let holding = ledger.gates.iter().any(|g| !gate_lapsed(g, &ctx.now));
    // A lapsed gate is announced once: the seed is back in ready (computed at
    // read time) and this durable event tells the crew. Its status stays
    // `blocked`, so `sd blocked --lapsed` keeps listing it until someone
    // re-gates, clears or works it.
    if external_done && !holding && ledger.gates.iter().any(|g| g.lapse_generation.is_none()) {
        ledger.generation = ledger
            .generation
            .checked_add(1)
            .ok_or_else(|| SdError::failed("unlock generation overflow"))?;
        let generation = ledger.generation;
        for g in &mut ledger.gates {
            g.lapse_generation.get_or_insert(generation);
        }
        ledger.events.push(UnlockEvent {
            event_id: event_id(id, generation),
            issue_id: id.into(),
            assignee: after.assignee.clone(),
            ready_generation: generation,
            observed_at: ctx.now.clone(),
            receipt: None,
        });
    }
    let done = external_done && ledger.gates.is_empty();
    if !(external_done && !holding) && after.status == "open" {
        after.status = "blocked".into();
        ledger.managed_status = true;
    }
    if done && ledger.managed_status && after.status == "blocked" && after.defer_until.is_none() {
        let ids: Vec<_> = after.blocked_on.iter().cloned().collect();
        let internal = b.snapshot_seeds(&ids)?;
        // Missing target coverage cannot certify readiness.
        let internal_done = ids.iter().all(|id| {
            internal
                .seeds
                .get(id)
                .is_some_and(|s| s.status == "closed" || s.is_tombstone())
        });
        if internal_done {
            after.status = "open".into();
            ledger.managed_status = false;
            ledger.generation = ledger
                .generation
                .checked_add(1)
                .ok_or_else(|| SdError::failed("unlock generation overflow"))?;
            ledger.events.push(UnlockEvent {
                event_id: event_id(id, ledger.generation),
                issue_id: id.into(),
                assignee: after.assignee.clone(),
                ready_generation: ledger.generation,
                observed_at: ctx.now.clone(),
                receipt: None,
            });
        }
    }
    if ledger == original && after.status == before.status {
        return Ok(None);
    }
    write(b, ctx, &before, after, &ledger).map(Some)
}

/// Persistent pending intents; only the shared consumer sends notifications.
pub fn events(b: &dyn Backend) -> Result<Vec<UnlockEvent>> {
    events_for(b, None)
}

/// A reaction reads only its target; a missing target is an error, not an empty page.
pub fn events_for(b: &dyn Backend, id: Option<&str>) -> Result<Vec<UnlockEvent>> {
    let snap = match id {
        Some(id) => {
            let snap = b.snapshot_seeds(&[id.into()])?;
            snap.get(id)?;
            snap
        }
        None => b.blocker_snapshot(None)?,
    };
    let mut out = Vec::new();
    for seed in snap.seeds.values() {
        let ledger = load(seed)?;
        // A lapsed gate's event is pending while its seed is still lapsed.
        let lapsed = seed.status == "blocked"
            && !ledger.gates.is_empty()
            && ledger.gates.iter().all(|g| g.lapse_generation.is_some());
        if (seed.status != "open" && !lapsed)
            || seed.defer_until.is_some()
            || !super::currently_done(b, seed, None)
        {
            continue;
        }
        let ids: Vec<_> = seed.blocked_on.iter().cloned().collect();
        let internal = b.snapshot_seeds(&ids)?;
        if !ids.iter().all(|id| {
            internal
                .seeds
                .get(id)
                .is_some_and(|s| s.status == "closed" || s.is_tombstone())
        }) {
            continue;
        }
        out.extend(
            ledger
                .events
                .into_iter()
                .filter(|event| event.receipt.is_none()),
        );
    }
    out.sort_by(|a, b| a.event_id.cmp(&b.event_id));
    Ok(out)
}

/// Record the shared notifier's retained receipt. Repeating the same ACK is a no-op.
pub fn receipt(b: &dyn Backend, id: &str, event_id: &str) -> Result<UnlockEvent> {
    let snap = b.snapshot_seeds(&[id.into()])?;
    load(snap.get(id)?)?
        .events
        .into_iter()
        .find(|event| event.event_id == event_id)
        .ok_or_else(|| SdError::not_found(event_id))
}

/// Record the shared notifier's retained receipt. Repeating the same ACK is a no-op.
pub fn ack(
    b: &mut dyn Backend,
    ctx: &Ctx,
    id: &str,
    event_id: &str,
    receipt: &str,
) -> Result<Option<u64>> {
    if receipt.trim().is_empty() {
        return Err(SdError::usage("ACK requires a retained delivery receipt"));
    }
    let snap = b.snapshot_seeds(&[id.into()])?;
    let before = snap.get(id)?;
    let mut ledger = load(before)?;
    let event = ledger
        .events
        .iter_mut()
        .find(|event| event.event_id == event_id)
        .ok_or_else(|| SdError::not_found(event_id))?;
    if let Some(existing) = &event.receipt {
        return if existing == receipt {
            Ok(None)
        } else {
            Err(SdError::conflict("unlock already has a different receipt"))
        };
    }
    event.receipt = Some(receipt.into());
    write(b, ctx, before, before.clone(), &ledger).map(Some)
}
