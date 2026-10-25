//! Read-only board consumer of the shared Quipu bootstrap and stream applier.
//!
//! The caller supplies a manifest and stream key obtained through its trusted
//! bootstrap channel. This adapter does not authenticate that channel or fetch
//! a snapshot. It owns the store and applier together; caller-provided progress
//! can never be substituted for the progress of the store being read.

use crate::backend::Ctx;
use crate::engine::{self, ListReq, Page, ReadyReq};
use crate::error::{Result, SdError};
use crate::projection::{AppliedProgress, Projected, SourceBinding};
use crate::quipu_backend::QuipuBackend;
use crate::vocab;
use quipu::applier::{Applier, Outcome};
use quipu::asof_share::{self, Manifest};
use quipu::Store;
use serde_json::Value as Json;
use std::collections::BTreeSet;

/// A replica that serves board reads only with verified, bounded freshness.
/// No writable backend or mutable store is exposed.
pub struct StreamBoard {
    backend: QuipuBackend,
    applier: Applier,
    manifest: Manifest,
    bootstrapped: bool,
}

impl StreamBoard {
    /// Prepare an empty replica for an independently trusted scoped manifest.
    /// The manifest must cover exactly the board and its ephemeral sibling.
    pub fn empty(
        graph: &str,
        manifest: Manifest,
        expected: &SourceBinding,
        key: &str,
    ) -> Result<Self> {
        let mut graphs = vec![graph.to_owned(), vocab::ephemeral_graph(graph)];
        graphs.sort();
        let binding = SourceBinding {
            store_id: manifest.source.store_id.clone(),
            log_epoch: manifest.source.log_epoch.clone(),
            scope_digest: manifest.scope_digest.clone(),
            policy_digest: manifest.policy_digest.clone(),
        };
        if &binding != expected
            || binding.store_id.is_empty()
            || binding.log_epoch.is_empty()
            || binding.policy_digest.is_empty()
            || manifest.scope != graphs
            || !manifest.context_graphs.is_empty()
            || manifest.scope_digest != asof_share::scope_digest(&graphs, &[])
            || manifest
                .completeness
                .graphs
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                != graphs
            || !manifest.completeness.valid_time
            || manifest.completeness.bytes != "lossless"
            || key.len() != 64
            || !key.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(SdError::refused(
                "replica bootstrap binding or completeness mismatch",
            ));
        }
        let store = Store::open_in_memory()?;
        let mut applier = Applier::open(&store)?;
        applier.trust_stream(key, &manifest.scope_digest);
        Ok(Self {
            backend: QuipuBackend::read_only(store, graph),
            applier,
            manifest,
            bootstrapped: false,
        })
    }

    /// Feed an unchanged transport frame to the ONE shared applier.
    /// Legacy unsigned change pages are deliberately not an input here.
    pub fn apply_frame(&mut self, event: &str, data: &str, now_ms: u64) -> Result<Outcome> {
        if data.len() > quipu::feed_parts::MAX_FRAME_BYTES {
            self.applier.end_session();
            return Err(SdError::refused(
                "replica frame exceeds the shared frame budget",
            ));
        }
        if event.starts_with("quipu.asof") {
            if self.bootstrapped {
                return Err(SdError::refused(
                    "rebuild the replica before another bootstrap",
                ));
            }
            if matches!(event, "quipu.asof" | "quipu.asof.begin") {
                let frame: Json = serde_json::from_str(data)
                    .map_err(|_| SdError::refused("invalid bootstrap frame"))?;
                if frame["manifest"]
                    != serde_json::to_value(&self.manifest)
                        .map_err(|_| SdError::refused("invalid bootstrap manifest"))?
                {
                    return Err(SdError::refused(
                        "bootstrap frame differs from trusted manifest",
                    ));
                }
            }
        } else if !event.starts_with("quipu.replicate.") || !self.bootstrapped {
            return Err(SdError::refused(
                "replica needs its committed bootstrap and signed stream",
            ));
        }
        let outcome = match self
            .applier
            .apply_frame(self.backend.store_mut(), event, data, now_ms)
        {
            Ok(o) => o,
            Err(e) => {
                self.applier.end_session();
                return Err(e.into());
            }
        };
        match &outcome {
            Outcome::Bootstrapped { source_tx } => {
                self.bootstrapped = *source_tx == self.manifest.watermark_tx;
            }
            Outcome::Rebootstrap { .. } | Outcome::Revoked { .. } | Outcome::Stopped { .. } => {
                self.bootstrapped = false;
                self.applier.end_session();
            }
            _ => {}
        }
        Ok(outcome)
    }

    /// Register a caller-generated nonce before its transport sends it.
    pub fn challenge(&mut self, nonce: [u8; 16], now_ms: u64) -> Result<String> {
        let session = self
            .applier
            .session()
            .ok_or_else(|| SdError::refused("replica stream is not open"))?
            .to_owned();
        self.applier
            .challenge(&session, nonce, now_ms)
            .map_err(|e| SdError::refused(&e))?;
        Ok(session)
    }

    /// A closed transport invalidates every freshness proof immediately.
    pub fn disconnect(&mut self) {
        self.applier.end_session();
    }

    /// Discard proofs after a caller-observed monotonic clock discontinuity.
    pub fn clock_discontinuity(&mut self) {
        self.applier.clock_discontinuity();
    }

    /// Build the next stream request from committed source progress, never a
    /// local transaction. A disconnect retains this cursor but no proof.
    pub fn stream_request(&self) -> Result<quipu::replicate_source::StreamRequest> {
        if !self.bootstrapped {
            return Err(SdError::refused("replica bootstrap is incomplete"));
        }
        let applied = self.applier.progress().map_or(0, |p| p.source_tx);
        Ok(quipu::replicate_source::StreamRequest {
            graphs: self.manifest.scope.clone(),
            context_graphs: self.manifest.context_graphs.clone(),
            since: applied.max(self.applier.coverage().unwrap_or(0)),
            expected_store_id: self.manifest.source.store_id.clone(),
        })
    }

    fn progress(&self, now_ms: u64, max_ms: u64) -> Result<(AppliedProgress, u64)> {
        let fresh = self.applier.freshness(now_ms);
        if !self.bootstrapped || max_ms == 0 || !fresh.within(max_ms) {
            return Err(SdError::refused("replica bounded freshness is UNKNOWN"));
        }
        let p = self
            .applier
            .progress()
            .ok_or_else(|| SdError::refused("replica progress is UNKNOWN"))?;
        if p.source.store_id != self.manifest.source.store_id
            || p.source.log_epoch != self.manifest.source.log_epoch
        {
            return Err(SdError::refused("replica source or epoch changed"));
        }
        let local_tx = u64::try_from(self.backend.store().transaction_head()?)
            .map_err(|_| SdError::refused("invalid local replica cursor"))?;
        let source_tx = u64::try_from(p.source_tx)
            .map_err(|_| SdError::refused("invalid source replica cursor"))?;
        Ok((
            AppliedProgress {
                source: SourceBinding {
                    store_id: p.source.store_id.clone(),
                    log_epoch: p.source.log_epoch.clone(),
                    scope_digest: self.manifest.scope_digest.clone(),
                    policy_digest: self.manifest.policy_digest.clone(),
                },
                source_tx,
                local_tx,
                graphs: self.manifest.scope.iter().cloned().collect::<BTreeSet<_>>(),
                complete: true,
                heartbeat_age_ms: fresh.last_frame_age_ms,
            },
            fresh
                .staleness_bound_ms
                .ok_or_else(|| SdError::refused("replica proof missing"))?,
        ))
    }

    /// Board or plate using the existing engine definition at a committed cut.
    pub fn list(&self, request: &ListReq, now_ms: u64, max_ms: u64) -> Result<Projected<Page>> {
        let (progress, bound) = self.progress(now_ms, max_ms)?;
        let value = engine::list(&self.backend, request, Some(progress.local_tx))?;
        Ok(Projected {
            value,
            progress,
            bounded_freshness_ms: Some(bound),
        })
    }

    /// Readiness remains advisory; claims always go to the authoritative writer.
    pub fn ready(
        &self,
        ctx: &Ctx,
        request: &ReadyReq,
        now_ms: u64,
        max_ms: u64,
    ) -> Result<Projected<Page>> {
        let (progress, bound) = self.progress(now_ms, max_ms)?;
        let value = engine::ready(&self.backend, ctx, request, Some(progress.local_tx))?;
        Ok(Projected {
            value,
            progress,
            bounded_freshness_ms: Some(bound),
        })
    }
}
