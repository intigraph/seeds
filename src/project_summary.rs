//! Owned static summary values from the governed store, not a client index.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The exact store and two-graph coverage vouched for by a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Main graph term ID in the serving store.
    pub project: i64,
    /// Ephemeral graph term ID in the same store.
    pub ephemeral: i64,
    /// Store identity, not an independent restore epoch.
    pub store_id: String,
    /// Producer layout and decoding contract.
    pub schema_id: String,
    /// Raw mutation generation, not a semantic transaction ID.
    pub generation: i64,
}
/// Static decoded counts; tombstones are separate from active dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Explicit reconciliation or transactional maintenance contract.
    pub maintenance: String,
    /// Active decoded seeds.
    pub total: u64,
    /// Decoded tombstones.
    pub tombstones: u64,
    /// Valid comments, including syntactically valid orphans.
    pub comments: u64,
    /// Active seeds by status, including unknown status names.
    pub by_status: BTreeMap<String, u64>,
    /// Active seeds by kind.
    pub by_kind: BTreeMap<String, u64>,
    /// Active seeds by priority.
    pub by_priority: BTreeMap<u8, u64>,
    /// Active seeds by decoded assignee.
    pub by_assignee: BTreeMap<String, u64>,
    /// Active seeds per distinct label, including the no-label placeholder.
    pub by_label: BTreeMap<String, u64>,
    /// Unsupported in this producer contract; must remain absent.
    pub dynamic_statistics: Option<serde_json::Value>,
    /// Unsupported in this producer contract; raw generation is not a substitute.
    pub semantic_transaction: Option<u64>,
}
/// One bounded static summary with its publication metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    /// Registered main graph IRI, bound by the server's metadata snapshot.
    pub project_iri: String,
    /// Registered ephemeral graph IRI from that same snapshot.
    pub ephemeral_iri: String,
    /// Coverage under which the counts were published.
    pub coverage: Coverage,
    /// Producer evaluation clock, not a transaction timestamp.
    pub evaluated_at: String,
    /// Earliest expiry; the server refuses expired summaries.
    pub valid_until: Option<String>,
    /// Static counts, bound by the server to retained decoded subjects.
    pub counts: Counts,
}

/// Publication identity without duplicating the maintained counts in output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Registered main graph.
    pub project_iri: String,
    /// Registered ephemeral graph.
    pub ephemeral_iri: String,
    /// Serving store identity, not a restore epoch.
    pub store_id: String,
    /// Static decoder and layout contract.
    pub schema_id: String,
    /// Raw source coverage generation.
    pub generation: i64,
    /// Evaluation clock.
    pub evaluated_at: String,
    /// Producer expiry bound.
    pub valid_until: Option<String>,
    /// Explicitly unsupported semantic transaction receipt.
    pub semantic_transaction: Option<u64>,
}
impl From<&ProjectSummary> for Metadata {
    fn from(summary: &ProjectSummary) -> Self {
        Self {
            project_iri: summary.project_iri.clone(),
            ephemeral_iri: summary.ephemeral_iri.clone(),
            store_id: summary.coverage.store_id.clone(),
            schema_id: summary.coverage.schema_id.clone(),
            generation: summary.coverage.generation,
            evaluated_at: summary.evaluated_at.clone(),
            valid_until: summary.valid_until.clone(),
            semantic_transaction: summary.counts.semantic_transaction,
        }
    }
}
