//! Read views over a committed replica supplied by the shared Quipu applier.
//!
//! This module neither tails a feed nor applies changes. The integration must
//! obtain the store and progress from the SAME committed applier snapshot. A
//! caller-supplied progress record is not independently authenticated here.
//! No CLI routing uses this boundary until that integration is available.

use crate::backend::Ctx;
use crate::engine::{self, ListReq, Page, ReadyReq};
use crate::error::{Result, SdError};
use crate::quipu_backend::QuipuBackend;
use crate::vocab;
use std::collections::BTreeSet;

/// The source and policy binding established by bootstrap and the applier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBinding {
    pub store_id: String,
    pub log_epoch: String,
    pub scope_digest: String,
    pub policy_digest: String,
}

/// Progress accompanying one fully committed applier snapshot.
#[derive(Clone, Debug)]
pub struct AppliedProgress {
    pub source: SourceBinding,
    /// Source transaction, NEVER a transaction number in the local store.
    pub source_tx: u64,
    /// Local commit containing BOTH the applied facts and this progress.
    /// Read queries pin this transaction; it is unrelated to `source_tx`.
    pub local_tx: u64,
    pub graphs: BTreeSet<String>,
    pub complete: bool,
    /// Age measured by the integration's monotonic clock at view capture.
    /// This is heartbeat liveness, not a bounded-staleness proof.
    pub heartbeat_age_ms: Option<u64>,
}

/// A result with explicit source progress, separate from local transactions.
#[derive(Clone, Debug)]
pub struct Projected<T> {
    pub value: T,
    pub progress: AppliedProgress,
    /// Heartbeat liveness alone proves no staleness bound. A future adapter
    /// must supply the shared applier's verified challenge before routing a
    /// bounded production read. This fixture boundary always returns None.
    pub bounded_freshness_ms: Option<u64>,
}

/// A short-lived read capability. Construct afresh for each command using the
/// applier's atomic capture and current heartbeat age. Never reuse across an
/// await, a disconnect, or a process restart. Claims and writes have no entry
/// point here: they continue to use the authoritative remote backend.
///
/// ```compile_fail
/// use seeds::{backend::Ctx, engine, projection::BoardProjection};
/// fn cannot_claim(view: &mut BoardProjection, ctx: &Ctx, id: String) {
///     engine::update(view, ctx, &[id], &engine::UpdateReq {
///         claim: true,
///         ..Default::default()
///     }).unwrap();
/// }
/// ```
pub struct BoardProjection {
    backend: QuipuBackend,
    progress: AppliedProgress,
    heartbeat_window_ms: u64,
}

impl BoardProjection {
    /// Admit an already committed view. Incomplete or unreadable bootstrap
    /// state is UNKNOWN, not an empty board. Scope includes ephemeral seeds,
    /// since existing board and ready semantics read both project graphs.
    pub fn from_applied(
        backend: QuipuBackend,
        progress: AppliedProgress,
        expected: &SourceBinding,
        heartbeat_window_ms: u64,
    ) -> Result<Self> {
        let binding = &progress.source;
        if [
            &binding.store_id,
            &binding.log_epoch,
            &binding.scope_digest,
            &binding.policy_digest,
        ]
        .iter()
        .any(|part| part.trim().is_empty())
            || binding != expected
        {
            return Err(SdError::refused(
                "projection source/epoch/scope/policy mismatch",
            ));
        }
        let graph = backend.graph_iri();
        if !progress.complete
            || !progress.graphs.contains(graph)
            || !progress.graphs.contains(&vocab::ephemeral_graph(graph))
        {
            return Err(SdError::refused("projection completeness is UNKNOWN"));
        }
        let head = u64::try_from(backend.store().transaction_head()?)
            .map_err(|_| SdError::refused("projection local watermark is invalid"))?;
        if progress.local_tx > head {
            return Err(SdError::refused(
                "projection local watermark is unavailable",
            ));
        }
        if heartbeat_window_ms == 0 {
            return Err(SdError::refused(
                "projection heartbeat window must be nonzero",
            ));
        }
        Ok(Self {
            backend,
            progress,
            heartbeat_window_ms,
        })
    }

    fn observe(&self, heartbeat_age_ms: Option<u64>) -> Result<AppliedProgress> {
        if heartbeat_age_ms.is_none_or(|age| age > self.heartbeat_window_ms) {
            return Err(SdError::refused("projection liveness is UNKNOWN"));
        }
        let mut progress = self.progress.clone();
        progress.heartbeat_age_ms = heartbeat_age_ms;
        Ok(progress)
    }

    /// Board or assignee-filtered plate. Uses the existing Seeds definition.
    pub fn list(
        &self,
        request: &ListReq,
        heartbeat_age_ms: Option<u64>,
    ) -> Result<Projected<Page>> {
        let progress = self.observe(heartbeat_age_ms)?;
        Ok(Projected {
            value: engine::list(&self.backend, request, Some(progress.local_tx))?,
            progress,
            bounded_freshness_ms: None,
        })
    }

    /// Readiness is advisory. It cannot authorize an offline claim.
    pub fn ready(
        &self,
        ctx: &Ctx,
        request: &ReadyReq,
        heartbeat_age_ms: Option<u64>,
    ) -> Result<Projected<Page>> {
        let progress = self.observe(heartbeat_age_ms)?;
        Ok(Projected {
            value: engine::ready(&self.backend, ctx, request, Some(progress.local_tx))?,
            progress,
            bounded_freshness_ms: None,
        })
    }
}
