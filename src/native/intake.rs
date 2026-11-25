//! Native create admission. Failure never degrades to an advisory or a write.
use crate::backend::{Backend, SeedQuery};
use crate::engine::CreateReq;
use crate::error::{Result, SdError};
use crate::intake::name_candidates;
use crate::vocab;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn check(
    b: &dyn Backend,
    req: &mut CreateReq,
    overrides: &[String],
    reason: Option<&str>,
) -> Result<()> {
    let query = SeedQuery {
        statuses: Some(
            ["open", "hooked", "in_progress", "blocked", "deferred"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        ..Default::default()
    };
    let snap = b.snapshot_where(&query)?;
    let active: BTreeMap<_, _> = snap
        .seeds
        .values()
        .filter(|s| query.matches(s))
        .map(|s| (vocab::item_iri(&s.id), s))
        .collect();
    let mut candidates = name_candidates(
        active.values().copied(),
        &req.title,
        req.description.as_deref(),
    )?;
    if let Some(remote) = b.creation_resolution(&req.title, req.description.as_deref())? {
        candidates.extend(
            remote
                .into_iter()
                .filter(|c| active.contains_key(&c.iri))
                .filter(|c| {
                    !c.matched_on.starts_with("canonical_name:")
                        || c.score >= crate::intake::NAME_THRESHOLD
                }),
        );
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.iri.cmp(&b.iri)));
    let mut seen = BTreeSet::new();
    candidates.retain(|c| seen.insert(c.iri.clone()));
    let ids: BTreeSet<_> = candidates
        .iter()
        .map(|c| active[&c.iri].id.as_str())
        .collect();
    let excused: BTreeSet<_> = overrides.iter().map(String::as_str).collect();
    if !excused.is_empty()
        && (reason.is_none_or(|r| r.trim().is_empty()) || !excused.is_subset(&ids))
    {
        return Err(SdError::usage("--new-despite must name a current duplicate candidate and --because must give a non-empty reason"));
    }
    let pending: Vec<_> = candidates
        .iter()
        .filter(|c| !excused.contains(active[&c.iri].id.as_str()))
        .collect();
    if !pending.is_empty() {
        let lines = pending
            .iter()
            .take(5)
            .map(|c| {
                let seed = active[&c.iri];
                format!(
                    "{} ({:.3}, {}): {} — comment on {} instead",
                    seed.id, c.score, c.matched_on, seed.title, seed.id
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(SdError::refused(format!("near-duplicate seed; no create sent:\n{lines}\nIf distinct, pass --new-despite <id> for each candidate and --because <why>.")));
    }
    if !excused.is_empty() {
        req.intake_comment = Some(
            serde_json::json!({"kind":"duplicate-intake-override", "new_despite":overrides,
            "because":reason.unwrap_or_default().trim(), "embedding_threshold":crate::intake::THRESHOLD,
            "name_threshold":crate::intake::NAME_THRESHOLD,
            "candidates":candidates})
            .to_string(),
        );
        req.deps
            .extend(overrides.iter().map(|id| format!("related:{id}")));
    }
    Ok(())
}

/// Advisory directive hint for a new seed that names none (aegis-8dda.9,
/// Stiwi 2026-11-22/24). Returns stderr warnings only; it never refuses.
pub(super) fn directive_hint(b: &dyn Backend, req: &CreateReq) -> Vec<String> {
    if !crate::intake::wants_directive_hint(req.issue_type.as_deref(), req.ephemeral)
        || crate::intake::names_directive(&req.title, req.description.as_deref())
    {
        return vec![];
    }
    let text = format!(
        "{}\n{}",
        req.title,
        req.description.as_deref().unwrap_or_default()
    );
    crate::intake::directive_hint_lines(
        b.directive_hints(text.trim(), crate::intake::DIRECTIVE_HINTS),
    )
}

/// Validate coverage before filtering global matches to active project seeds.
pub(super) fn decode_resolution(body: &str) -> Result<Vec<quipu::resolution::EntityCandidate>> {
    let result: quipu::resolution::ResolutionResult = serde_json::from_str(body).map_err(|e| {
        SdError::refused(format!(
            "duplicate intake resolution UNKNOWN: {e}; no create sent"
        ))
    })?;
    if result.vector_scope.is_partial()
        || result.candidates.len() >= crate::intake::CANDIDATE_CAP
        || result.has_matches == result.candidates.is_empty()
        || result.candidates.iter().any(|c| {
            !c.score.is_finite()
                || !(crate::intake::THRESHOLD..=1.0).contains(&c.score)
                || !c.iri.starts_with("http")
                || c.matched_on.is_empty()
        })
    {
        return Err(SdError::refused(
            "duplicate intake resolution incomplete or invalid; no create sent",
        ));
    }
    Ok(result.candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Ctx, WriteBatch};
    use crate::model::Snapshot;
    use crate::quipu_backend::QuipuBackend;
    use quipu::resolution::EntityCandidate;
    #[test]
    fn incomplete_or_invalid_remote_results_never_admit_creation() {
        let good = serde_json::json!({"has_matches":false,"candidates":[],"vector_scope":{"kind":"whole_store"}});
        assert!(decode_resolution(&good.to_string()).unwrap().is_empty());
        for change in [
            serde_json::json!({"error":"unavailable"}),
            serde_json::json!({"has_matches":true,"candidates":[],"vector_scope":{"kind":"whole_store"}}),
            serde_json::json!({"has_matches":false,"candidates":[],"vector_scope":{"kind":"local_only","attached_layers":1}}),
            serde_json::json!({"has_matches":true,"candidates":[{"iri":"https://seeds.local/item/test-1","score":1.01,"matched_on":"embedding"}],"vector_scope":{"kind":"whole_store"}}),
            serde_json::json!({"has_matches":true,"candidates": vec![serde_json::json!({"iri":"https://seeds.local/item/test-1","score":0.99,"matched_on":"embedding"}); crate::intake::CANDIDATE_CAP],"vector_scope":{"kind":"whole_store"}}),
        ] {
            assert!(decode_resolution(&change.to_string()).is_err());
        }
    }
    struct Remote {
        inner: QuipuBackend,
        candidate: String,
        fail: bool,
    }
    impl Backend for Remote {
        fn ready_ids(&self, at: Option<u64>) -> Result<Vec<String>> {
            self.inner.ready_ids(at)
        }
        fn claims_of(&self, id: &str) -> Result<Vec<(u64, crate::backend::Claims)>> {
            self.inner.claims_of(id)
        }
        fn snapshot(&self, at: Option<u64>) -> Result<Snapshot> {
            self.inner.snapshot(at)
        }
        fn commit(&mut self, batch: &WriteBatch, ctx: &Ctx) -> Result<u64> {
            self.inner.commit(batch, ctx)
        }
        fn creation_resolution(
            &self,
            _title: &str,
            description: Option<&str>,
        ) -> Result<Option<Vec<EntityCandidate>>> {
            assert_eq!(description, Some("same underlying problem"));
            if self.fail {
                return Err(SdError::refused("resolver unavailable"));
            }
            Ok(Some(vec![EntityCandidate {
                iri: self.candidate.clone(),
                score: 0.96,
                matched_on: "embedding:0.96".into(),
            }]))
        }
    }
    #[test]
    fn semantic_candidate_refuses_even_when_title_is_different_and_failure_stays_unknown() {
        let mut b = QuipuBackend::in_memory("https://seeds.local/project/fixture").unwrap();
        let ctx = Ctx {
            now: "2026-11-24T12:00:00Z".into(),
            actor: "tester".into(),
            prefix: "fixture".into(),
            claims: Default::default(),
        };
        let (existing, _) = crate::engine::create(
            &mut b,
            &ctx,
            &CreateReq {
                title: "Database connection pool exhaustion".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut remote = Remote {
            inner: b,
            candidate: vocab::item_iri(&existing.id),
            fail: false,
        };
        let mut req = CreateReq {
            title: "Requests hang under load".into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let error = check(&remote, &mut req, &[], None).unwrap_err();
        assert!(error.message.contains(&existing.id));
        remote.fail = true;
        assert!(check(&remote, &mut req, &[], None)
            .unwrap_err()
            .message
            .contains("unavailable"));
        assert_eq!(remote.inner.snapshot(None).unwrap().seeds.len(), 1);
    }
}
