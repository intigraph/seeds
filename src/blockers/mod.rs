//! Declared external prerequisites and a persistent unlock outbox.
//!
//! Metadata is stored on the existing Action, so no foreign node is invented
//! or retyped. The ordinary write provenance and revision CAS cover changes.
use crate::backend::Backend;
use crate::error::{Result, SdError};
use crate::model::{Obj, Seed};
use crate::vocab;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub mod condition;
pub mod engine;
pub mod migration;

/// A read's outcome. UNKNOWN is never a completed prerequisite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A valid condition read returned false.
    Pending,
    /// A valid condition read returned true.
    Done,
    /// The condition could not be read or interpreted.
    Unknown,
}

/// One declared external dependency, including the author's provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    /// Exact target IRI.
    pub iri: String,
    /// Explicit dataset, or ROOT when absent.
    pub graph: Option<String>,
    /// Read-only SPARQL ASK declaring completion.
    pub ask: String,
    /// The chosen condition name, or `ask` for an explicit condition.
    pub condition: String,
    /// Last reconciled outcome.
    pub state: State,
    /// When that outcome last changed; not a polling heartbeat.
    pub observed_at: Option<String>,
    /// Creation actor, declared by the writer.
    pub actor: String,
    /// Creation time from the caller's Ctx.
    pub created_at: String,
}

/// Persistent notification intent. Only the shared notifier performs delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlockEvent {
    /// Stable id across reads, retries and ordinary seed edits.
    pub event_id: String,
    /// The affected seed.
    pub issue_id: String,
    /// Assignee at the unlock; None is the shared notifier's administrator route.
    pub assignee: Option<String>,
    /// Persisted readiness episode.
    pub ready_generation: u64,
    /// Time of the observed transition.
    pub observed_at: String,
    /// Verified delivery receipt recorded by the shared consumer.
    pub receipt: Option<String>,
}

/// Versioned blocker state and its outbox, preserved by older readers as extras.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    /// Encoding version; an unknown version refuses rather than losing a gate.
    pub version: u8,
    /// Active edges indexed by exact target IRI.
    pub blockers: BTreeMap<String, Blocker>,
    /// Whether this feature changed open to blocked and may restore it.
    pub managed_status: bool,
    /// Monotonic readiness counter; ordinary edits do not change it.
    pub generation: u64,
    /// Pending and acknowledged intent, retaining receipt history.
    pub events: Vec<UnlockEvent>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: 1,
            blockers: BTreeMap::new(),
            managed_status: false,
            generation: 0,
            events: Vec::new(),
        }
    }
}

/// The persistent state predicate.
pub fn ledger_predicate() -> String {
    vocab::seeds("externalBlockerState")
}
/// Direct graph edge to each external node, independent of seed-only blockedOn.
pub fn edge_predicate() -> String {
    vocab::seeds("externalBlockedOn")
}
/// Whether a seed declares this feature, even if its encoding is unreadable.
pub fn has_metadata(seed: &Seed) -> bool {
    seed.extra
        .iter()
        .any(|(p, _)| p == &ledger_predicate() || p == &edge_predicate())
}

/// Decode and validate a complete declaration; partial metadata is UNKNOWN.
pub fn load(seed: &Seed) -> Result<Ledger> {
    let values: Vec<_> = seed
        .extra
        .iter()
        .filter(|(p, _)| p == &ledger_predicate())
        .collect();
    if values.is_empty() && !has_metadata(seed) {
        return Ok(Ledger::default());
    }
    let [(_, Obj::Str(text))] = values.as_slice() else {
        return Err(SdError::refused(
            "external blocker state is missing, duplicated or not text",
        ));
    };
    let ledger: Ledger = serde_json::from_str(text).map_err(|_| {
        SdError::refused("external blocker state is unreadable; prerequisites remain unresolved")
    })?;
    if ledger.version != 1 {
        return Err(SdError::refused(
            "unsupported external blocker state version",
        ));
    }
    let edges: BTreeSet<_> = seed
        .extra
        .iter()
        .filter(|(p, _)| p == &edge_predicate())
        .map(|(_, o)| match o {
            Obj::Iri(i) => Ok(i.clone()),
            _ => Err(SdError::refused("external blocker edge is not an IRI")),
        })
        .collect::<Result<_>>()?;
    if edges != ledger.blockers.keys().cloned().collect() {
        return Err(SdError::refused(
            "external blocker edges and declarations disagree",
        ));
    }
    for (iri, blocker) in &ledger.blockers {
        condition::validate_iri(iri)?;
        if iri != &blocker.iri || blocker.ask.len() > 16_384 {
            return Err(SdError::refused("invalid external blocker declaration"));
        }
        if let Some(graph) = &blocker.graph {
            condition::validate_iri(graph)?;
        }
    }
    let mut ids = BTreeSet::new();
    for event in &ledger.events {
        if event.issue_id != seed.id
            || event.ready_generation > ledger.generation
            || !ids.insert(&event.event_id)
            || event.event_id != event_id(&seed.id, event.ready_generation)
        {
            return Err(SdError::refused("invalid or duplicated unlock intent"));
        }
    }
    Ok(ledger)
}

/// Write only this feature's predicates; unrelated extra facts are untouched.
pub fn save(seed: &mut Seed, ledger: &Ledger) -> Result<()> {
    let text = serde_json::to_string(ledger)
        .map_err(|_| SdError::failed("cannot encode blocker state"))?;
    seed.extra
        .retain(|(p, _)| p != &ledger_predicate() && p != &edge_predicate());
    seed.extra.insert((ledger_predicate(), Obj::Str(text)));
    for iri in ledger.blockers.keys() {
        seed.extra.insert((edge_predicate(), Obj::Iri(iri.clone())));
    }
    load(seed)?;
    Ok(())
}

/// Stable event identity, including the readiness generation.
pub fn event_id(id: &str, generation: u64) -> String {
    format!("seeds:unlock:{id}:{generation}")
}

/// Reconciled unresolved targets. Malformed metadata cannot make a seed ready.
pub fn unresolved(seed: &Seed) -> Vec<String> {
    match load(seed) {
        Ok(ledger) => ledger
            .blockers
            .values()
            .filter(|b| b.state != State::Done)
            .map(|b| b.iri.clone())
            .collect(),
        Err(_) => vec!["UNKNOWN external blocker metadata".into()],
    }
}

/// Current read-side readiness, paired with the persisted reconciliation state.
pub fn currently_done(backend: &dyn Backend, seed: &Seed, at: Option<u64>) -> bool {
    match load(seed) {
        Ok(ledger) => ledger
            .blockers
            .values()
            .all(|b| b.state == State::Done && condition::observe(backend, b, at) == State::Done),
        Err(_) => false,
    }
}

/// External edges for display, retaining UNKNOWN when observation fails.
pub fn display(backend: &dyn Backend, seed: &Seed, at: Option<u64>) -> serde_json::Value {
    match load(seed) {
        Ok(ledger) => {
            serde_json::json!({"status":"exact", "blockers":ledger.blockers.values().map(|blocker| {
            serde_json::json!({"iri":blocker.iri,"graph":blocker.graph,"condition":blocker.condition,
                "state":condition::observe(backend, blocker, at),"recorded_state":blocker.state,
                "observed_at":blocker.observed_at,"actor":blocker.actor,"created_at":blocker.created_at})
        }).collect::<Vec<_>>()})
        }
        Err(_) => serde_json::json!({"status":"unknown","blockers":[]}),
    }
}
