//! Duplicate intake reuses Quipu's episode resolver, without writing the board.
use crate::error::{Result, SdError};
use crate::model::Seed;
use quipu::resolution::EntityCandidate;
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};

/// Embedding similarity; a score is not a probability.
pub const THRESHOLD: f64 = 0.92;
/// Long tracker templates can have high Jaro scores despite different targets.
/// The retained replay flagged 119/533 at .92, including different episode IDs;
/// .99 retained 88, with those inspected template-only matches removed.
pub const NAME_THRESHOLD: f64 = 0.99;
/// A full result at this bound is incomplete and must not authorize creation.
pub const CANDIDATE_CAP: usize = 1000;

/// Local/offline name matching over exactly the supplied active corpus. The
/// scratch store has no embedding provider; remote intake adds the configured
/// server's embedding half. Never represent this as embedding coverage.
pub fn name_candidates<'a>(
    seeds: impl IntoIterator<Item = &'a Seed>,
    title: &str,
    description: Option<&str>,
) -> Result<Vec<EntityCandidate>> {
    let mut store = Store::open_in_memory().map_err(graph_error)?;
    let label = store
        .intern(crate::vocab::RDFS_LABEL)
        .map_err(graph_error)?;
    let mut facts = Vec::new();
    for seed in seeds {
        if matches!(seed.status.as_str(), "closed" | "tombstone") {
            continue;
        }
        facts.push(Datum {
            entity: store
                .intern(&crate::vocab::item_iri(&seed.id))
                .map_err(graph_error)?,
            attribute: label,
            value: Value::Str(seed.title.clone()),
            valid_from: "1970-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    if !facts.is_empty() {
        store
            .transact(
                &facts,
                "1970-01-01T00:00:00Z",
                None,
                Some("seeds:intake-preview"),
            )
            .map_err(graph_error)?;
    }
    let properties = description
        .map(|d| vec![("description".into(), d.into())])
        .unwrap_or_default();
    let result = quipu::resolve_entity(
        &store,
        title.trim(),
        &properties,
        NAME_THRESHOLD,
        CANDIDATE_CAP,
    )
    .map_err(graph_error)?;
    if result.candidates.len() >= CANDIDATE_CAP {
        return Err(SdError::refused(
            "duplicate intake candidate bound reached; no create sent",
        ));
    }
    Ok(result.candidates)
}

fn graph_error(error: quipu::error::Error) -> SdError {
    SdError::failed(format!("duplicate intake resolver: {error}"))
}
