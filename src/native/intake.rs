//! Native create admission: the refusal gate, plus advisory duplicate hints
//! (aegis-8dda.12). Without hint mode (`sd q`, bulk creates) a resolver
//! failure refuses. In hint mode it is recorded as UNKNOWN, never as "no
//! duplicates", and the create proceeds with a warning that names what was
//! not checked: the remote (embedding) half of the refusal gate as well as
//! the hints. The local canonical-name gate (0.99) still applies, and
//! `--strict` restores fail-closed. That degraded path is the item's own
//! acceptance test ("a Quipu timeout leaves the hint UNKNOWN and the create
//! proceeds with a warning"); see `docs/book/src/duplicate-intake.md`.
use super::cli::DupesCommand;
use crate::backend::{Backend, Ctx, SeedQuery};
use crate::engine::CreateReq;
use crate::error::{Result, SdError};
use crate::intake::{distinct_partners, hint_eligible, name_candidates, Hint};
use crate::model::{Obj, Seed};
use crate::vocab;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn check(
    b: &dyn Backend,
    req: &mut CreateReq,
    overrides: &[String],
    reason: Option<&str>,
) -> Result<()> {
    admit(b, req, overrides, reason, None).map(|_| ())
}

/// Hint mode (aegis-8dda.12): besides the refusal gate, report and record
/// the nearest WorkItems, open or closed within 30 days.
pub(super) struct HintMode<'a> {
    /// `ctx.now`, for the closed window.
    pub now: &'a str,
    /// Refuse when a hint is found or the hint lookup is UNKNOWN.
    pub strict: bool,
    /// Pairs to record as `quipu:distinctFrom` and never hint.
    pub distinct_from: &'a [String],
    /// An existing seed being re-hinted after an edit: never its own
    /// candidate, and the refusal gate does not apply (the edit is done).
    pub existing: Option<&'a str>,
}

/// What hint mode found: the candidates, and why the lookup is UNKNOWN.
pub(super) type HintResult = (Vec<Hint>, Option<String>);

const ACTIVE: [&str; 5] = ["open", "hooked", "in_progress", "blocked", "deferred"];

pub(super) fn admit(
    b: &dyn Backend,
    req: &mut CreateReq,
    overrides: &[String],
    reason: Option<&str>,
    hint: Option<&HintMode>,
) -> Result<Option<HintResult>> {
    let query = SeedQuery {
        statuses: Some(ACTIVE.into_iter().map(str::to_string).collect()),
        ..Default::default()
    };
    // Bodies are never read here: they were 90% of the open seeds' bytes
    // (~5 MB on every create, aegis-10m).
    let mut snap = b.snapshot_where_brief(&query)?;
    // Hint mode also reads seeds closed inside the window, bounded in the
    // query (never every closed seed); the window is re-applied below.
    let now = hint.and_then(|h| crate::engine::epoch_of(h.now));
    // The closed half is best effort (aegis-n0wc): over the cap, or on a
    // failed read, it is skipped and the hint is UNKNOWN, never a refusal.
    let mut closed_unknown = None;
    if let Some(now) = now {
        let cutoff = crate::engine::format_instant(now - crate::intake::HINT_CLOSED_DAYS * 86_400);
        match b.snapshot_closed_since_capped(Some(&cutoff[..10]), crate::intake::HINT_CLOSED_CAP) {
            Ok(Some(closed)) => {
                for (id, seed) in closed.seeds {
                    snap.seeds.entry(id).or_insert(seed);
                }
            }
            Ok(None) => {
                closed_unknown = Some(format!(
                    "{}: more than {} closed in the last {} days",
                    crate::intake::HINT_CLOSED_PARTIAL,
                    crate::intake::HINT_CLOSED_CAP,
                    crate::intake::HINT_CLOSED_DAYS
                ));
            }
            Err(e) => {
                closed_unknown = Some(format!(
                    "{}: {}",
                    crate::intake::HINT_CLOSED_PARTIAL,
                    e.message
                ));
            }
        }
    }
    let active: BTreeMap<_, _> = snap
        .seeds
        .values()
        .filter(|s| query.matches(s) && ACTIVE.contains(&s.status.as_str()))
        .map(|s| (vocab::item_iri(&s.id), s))
        .collect();
    let gate = hint.is_none_or(|h| h.existing.is_none());
    let mut candidates = if gate {
        name_candidates(
            active.values().copied(),
            &req.title,
            req.description.as_deref(),
        )?
    } else {
        vec![]
    };
    // In hint mode an unavailable resolver is UNKNOWN, never "no duplicates",
    // and the create proceeds with a warning (aegis-8dda.12); without hint
    // mode (quick and bulk creates) it still refuses.
    let mut unknown = None;
    let remote = match b.creation_resolution(&req.title, req.description.as_deref()) {
        Ok(remote) => remote.unwrap_or_default(),
        Err(e) if hint.is_some() => {
            unknown = Some(e.message);
            vec![]
        }
        Err(e) => return Err(e),
    };
    // A skipped closed half degrades the hint the same way (hint mode only:
    // `closed_unknown` is set only when `now` came from a hint).
    if let Some(closed) = closed_unknown {
        unknown = Some(match unknown {
            Some(resolver) => format!("{resolver}; {closed}"),
            None => closed,
        });
    }
    if gate {
        candidates.extend(
            remote
                .iter()
                .filter(|c| active.contains_key(&c.iri))
                .filter(|c| {
                    !c.matched_on.starts_with("canonical_name:")
                        || c.score >= crate::intake::NAME_THRESHOLD
                })
                .cloned(),
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
    let Some(hint) = hint else {
        return Ok(None);
    };
    // The hinted side is filtered like the corpus: an inbox pointer,
    // ephemeral seed or machine record is never hinted, recorded, listed by
    // `sd dupes` or refused by `--strict` for a hint. Only an UNKNOWN
    // lookup still matters to it, because the refusal gate was degraded.
    let subject = hint.existing.is_some()
        || crate::intake::hint_subject(
            &req.title,
            &req.labels,
            req.issue_type.as_deref(),
            req.ephemeral,
        );
    let corpus: BTreeMap<_, _> = snap
        .seeds
        .values()
        .filter(|s| {
            (ACTIVE.contains(&s.status.as_str()) || s.status == "closed") && hint_eligible(s, now)
        })
        .map(|s| (vocab::item_iri(&s.id), s))
        .collect();
    // Never hint a pair already judged: overridden gate candidates (now
    // related), recorded or requested distinctFrom, or the seed itself.
    let mut judged: BTreeSet<String> = excused.iter().map(|s| s.to_string()).collect();
    judged.extend(hint.distinct_from.iter().cloned());
    if let Some(me) = hint.existing {
        judged.insert(me.to_string());
        judged.extend(distinct_partners(snap.seeds.values(), me));
    }
    let mut found = crate::intake::name_candidates_at(
        corpus.values().copied(),
        &req.title,
        req.description.as_deref(),
        crate::intake::HINT_NAME_THRESHOLD,
    )?;
    // Remote name matches get the same hint floor as local ones; embedding
    // candidates are already at Quipu's resolver threshold.
    found.extend(remote.into_iter().filter(|c| {
        corpus.contains_key(&c.iri)
            && (!c.matched_on.starts_with("canonical_name:")
                || c.score >= crate::intake::HINT_NAME_THRESHOLD)
    }));
    found.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.iri.cmp(&b.iri)));
    let mut seen = BTreeSet::new();
    found.retain(|c| seen.insert(c.iri.clone()));
    let hints: Vec<Hint> = found
        .into_iter()
        .filter(|c| subject && !judged.contains(&corpus[&c.iri].id))
        .take(crate::intake::HINT_TOP)
        .map(|c| {
            let s = corpus[&c.iri];
            Hint {
                id: s.id.clone(),
                title: s.title.clone(),
                status: s.status.clone(),
                score: c.score,
                matched_on: c.matched_on,
            }
        })
        .collect();
    if !hint.distinct_from.is_empty() {
        // Each --distinct-from must name a seed that exists.
        let known = b.snapshot_items(hint.distinct_from)?;
        for id in hint.distinct_from {
            known.get(id)?;
        }
    }
    if hint.strict && (unknown.is_some() || !hints.is_empty()) {
        let mut lines = crate::intake::hint_lines(&hints, None, crate::intake::HintContext::Create);
        if let Some(why) = &unknown {
            lines.insert(0, format!("duplicate check UNKNOWN ({why})"));
        }
        let lines = lines.join("\n");
        return Err(SdError::refused(format!(
            "--strict: possible duplicate or UNKNOWN duplicate check; no create sent:\n{lines}"
        )));
    }
    if subject {
        req.extra
            .extend(crate::intake::hint_facts(&hints, unknown.as_deref()));
    }
    req.extra.extend(hint.distinct_from.iter().map(|id| {
        (
            crate::intake::DISTINCT_FROM.to_string(),
            crate::model::Obj::Iri(vocab::item_iri(id)),
        )
    }));
    Ok(Some((hints, unknown)))
}

/// Re-run duplicate hints for an edited seed and replace its recorded hint
/// facts (aegis-8dda.12). Returns the stderr lines.
pub(super) fn rehint(b: &mut dyn Backend, ctx: &Ctx, seed: &Seed) -> Result<Vec<String>> {
    // Only live work is re-hinted: never a closed seed (`sd dupes` does not
    // list it), an inbox pointer, an ephemeral seed or a machine record.
    if seed.status == "closed" || !hint_eligible(seed, None) {
        return Ok(vec![]);
    }
    let mut req = CreateReq {
        title: seed.title.clone(),
        description: seed.description.clone(),
        ..Default::default()
    };
    let mode = HintMode {
        now: &ctx.now,
        strict: false,
        distinct_from: &[],
        existing: Some(&seed.id),
    };
    let Some((hints, unknown)) = admit(b, &mut req, &[], None, Some(&mode))? else {
        return Ok(vec![]);
    };
    crate::engine::amend_extra(
        b,
        ctx,
        &seed.id,
        &|p, _| crate::intake::is_hint_predicate(p),
        req.extra,
        "seeds:duplicate-hint",
    )?;
    Ok(crate::intake::hint_lines(
        &hints,
        unknown.as_deref(),
        crate::intake::HintContext::Edit,
    ))
}

/// `sd dupes`: list open hint pairs; `sd dupes resolve`: settle one.
pub(super) fn dupes(
    b: &mut dyn Backend,
    ctx: &Ctx,
    command: Option<&DupesCommand>,
    json: bool,
) -> Result<super::Outcome> {
    match command {
        None => {
            let pairs = crate::intake::open_hint_pairs(&b.snapshot(None)?);
            let text = if pairs.is_empty() {
                "no open duplicate hints".to_string()
            } else {
                pairs
                    .iter()
                    .map(|p| {
                        format!(
                            "{} [{}] {}\n  may duplicate {} [{}] {}",
                            p.id,
                            p.status,
                            p.title,
                            p.duplicate_of,
                            p.duplicate_of_status.as_deref().unwrap_or("unreadable"),
                            p.duplicate_of_title.as_deref().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(super::ok(json, serde_json::json!(pairs), text, vec![]))
        }
        Some(DupesCommand::Resolve {
            id,
            of,
            duplicate,
            distinct,
        }) => {
            if !duplicate && !distinct {
                return Err(SdError::usage(
                    "sd dupes resolve needs --duplicate or --distinct",
                ));
            }
            if id == of {
                return Err(SdError::usage("a seed cannot duplicate itself"));
            }
            let snap = b.snapshot_items(&[id.clone(), of.clone()])?;
            snap.get(id)?;
            snap.get(of)?;
            let target = vocab::item_iri(of);
            let hint = crate::intake::possible_duplicate_of();
            if *duplicate {
                let reason = format!("duplicate of {of}");
                let (seeds, tx, warnings) = crate::engine::close_as(
                    b,
                    ctx,
                    std::slice::from_ref(id),
                    Some(&reason),
                    Some("superseded"),
                    false,
                    None,
                )?;
                let text = format!("closed {id} as a duplicate of {of}");
                return Ok(super::ok(
                    json,
                    crate::output::with_tx(crate::output::seeds_json(&seeds), Some(tx)),
                    text,
                    warnings,
                ));
            }
            let (seed, tx) = crate::engine::amend_extra(
                b,
                ctx,
                id,
                &|p, o| p == hint && matches!(o, Obj::Iri(t) if t == &target),
                vec![(
                    crate::intake::DISTINCT_FROM.to_string(),
                    Obj::Iri(target.clone()),
                )],
                "seeds:duplicate-distinct",
            )?;
            Ok(super::ok(
                json,
                crate::output::with_tx(seed.to_json(), Some(tx)),
                format!("recorded {id} distinctFrom {of}"),
                vec![],
            ))
        }
    }
}

/// Advisory directive hint for a new seed that names none (aegis-8dda.9,
/// Stiwi 2026-11-22/24). Returns stderr warnings only; it never refuses.
/// A directive hint asked before the create's other reads, so its
/// `/hybrid_search` runs alongside the duplicate gate (aegis-n0wc).
pub(super) enum DirectiveHint {
    /// The seed needs no hint (machine record, ephemeral, names a directive).
    Skip,
    /// The lookup is running on its own thread.
    Started(crate::backend::DirectiveHintsJob),
    /// The backend has no concurrent form; ask in line when finishing.
    Later(String),
}

pub(super) fn start_directive_hint(b: &dyn Backend, req: &CreateReq) -> DirectiveHint {
    if !crate::intake::wants_directive_hint(req.issue_type.as_deref(), req.ephemeral)
        || crate::intake::names_directive(&req.title, req.description.as_deref())
    {
        return DirectiveHint::Skip;
    }
    let text = format!(
        "{}\n{}",
        req.title,
        req.description.as_deref().unwrap_or_default()
    );
    let text = text.trim().to_string();
    match b.spawn_directive_hints(&text, crate::intake::DIRECTIVE_HINTS) {
        Some(job) => DirectiveHint::Started(job),
        None => DirectiveHint::Later(text),
    }
}

/// The stderr lines of a started hint. A lookup that failed or panicked is
/// UNKNOWN and never affects the create.
pub(super) fn finish_directive_hint(b: &dyn Backend, pending: DirectiveHint) -> Vec<String> {
    let hints = match pending {
        DirectiveHint::Skip => return vec![],
        DirectiveHint::Started(job) => job
            .join()
            .unwrap_or_else(|_| Err(SdError::failed("directive hints: lookup thread panicked"))),
        DirectiveHint::Later(text) => b.directive_hints(&text, crate::intake::DIRECTIVE_HINTS),
    };
    crate::intake::directive_hint_lines(hints)
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

    #[test]
    fn a_failed_or_panicked_directive_hint_thread_is_unknown_never_an_error() {
        use super::*;
        let b =
            crate::quipu_backend::QuipuBackend::in_memory("https://seeds.local/project/fixture")
                .unwrap();
        let failed = DirectiveHint::Started(std::thread::spawn(|| {
            Err(SdError::failed("directive hints: HTTP 503"))
        }));
        let lines = finish_directive_hint(&b, failed);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("directive hints UNKNOWN"), "{lines:?}");
        assert!(lines[0].contains("HTTP 503"), "{lines:?}");
        let panicked = DirectiveHint::Started(std::thread::spawn(|| panic!("boom")));
        let lines = finish_directive_hint(&b, panicked);
        assert!(lines[0].contains("lookup thread panicked"), "{lines:?}");
        let found = DirectiveHint::Started(std::thread::spawn(|| {
            Ok(Some(vec![("https://x/stiwi-d".to_string(), 0.5)]))
        }));
        let lines = finish_directive_hint(&b, found);
        assert!(
            lines.iter().any(|l| l.contains("stiwi-d (0.50)")),
            "{lines:?}"
        );
        assert!(finish_directive_hint(&b, DirectiveHint::Skip).is_empty());
    }
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
        /// Answer a remote canonical-name match at this score instead of
        /// an embedding match.
        name_score: Option<f64>,
        fail: bool,
        /// Answer the capped closed-since read as over the cap (aegis-n0wc).
        closed_over_cap: bool,
    }
    impl Backend for Remote {
        fn snapshot_closed_since_capped(
            &self,
            since: Option<&str>,
            cap: usize,
        ) -> Result<Option<Snapshot>> {
            if self.closed_over_cap {
                return Ok(None);
            }
            self.inner.snapshot_closed_since_capped(since, cap)
        }
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
            Ok(Some(vec![match self.name_score {
                Some(score) => EntityCandidate {
                    iri: self.candidate.clone(),
                    score,
                    matched_on: format!("canonical_name:jaro_winkler:{score:.2}"),
                },
                None => EntityCandidate {
                    iri: self.candidate.clone(),
                    score: 0.96,
                    matched_on: "embedding:0.96".into(),
                },
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
            name_score: None,
            inner: b,
            candidate: vocab::item_iri(&existing.id),
            fail: false,
            closed_over_cap: false,
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

    fn ctx() -> Ctx {
        Ctx {
            now: "2026-11-27T05:00:00Z".into(),
            actor: "tester".into(),
            prefix: "fixture".into(),
            claims: Default::default(),
        }
    }

    fn board(titles: &[&str]) -> (QuipuBackend, Vec<String>) {
        let mut b = QuipuBackend::in_memory("https://seeds.local/project/fixture").unwrap();
        let ids = titles
            .iter()
            .map(|t| {
                crate::engine::create(
                    &mut b,
                    &ctx(),
                    &CreateReq {
                        title: (*t).into(),
                        ..Default::default()
                    },
                )
                .unwrap()
                .0
                .id
            })
            .collect();
        (b, ids)
    }

    fn mode<'a>(distinct: &'a [String], strict: bool) -> HintMode<'a> {
        HintMode {
            now: "2026-11-27T05:00:00Z",
            strict,
            distinct_from: distinct,
            existing: None,
        }
    }

    const OLD: &str = "StopFailure hook drops the session report";
    const NEW: &str = "StopFailure hook drops session reports";

    fn hinted(
        b: &QuipuBackend,
        distinct: &[String],
        strict: bool,
    ) -> Result<(CreateReq, HintResult)> {
        let mut req = CreateReq {
            title: NEW.into(),
            ..Default::default()
        };
        let found = admit(b, &mut req, &[], None, Some(&mode(distinct, strict)))?.unwrap();
        Ok((req, found))
    }

    #[test]
    fn a_near_copy_of_an_open_item_is_hinted_recorded_and_listed() {
        let (mut b, ids) = board(&[OLD, "Rotate the backup keys"]);
        let (req, (hints, unknown)) = hinted(&b, &[], false).unwrap();
        assert!(unknown.is_none());
        assert_eq!(hints.len(), 1, "{hints:?}");
        assert_eq!(hints[0].id, ids[0]);
        assert!(hints[0].score >= crate::intake::HINT_NAME_THRESHOLD);
        assert!(req.extra.contains(&(
            crate::intake::possible_duplicate_of(),
            Obj::Iri(vocab::item_iri(&ids[0]))
        )));
        let (new, _) = crate::engine::create(&mut b, &ctx(), &req).unwrap();
        let pairs = crate::intake::open_hint_pairs(&b.snapshot(None).unwrap());
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (pairs[0].id.as_str(), pairs[0].duplicate_of.as_str()),
            (new.id.as_str(), ids[0].as_str())
        );
        // Resolving as distinct clears the pair for good.
        dupes(
            &mut b,
            &ctx(),
            Some(&DupesCommand::Resolve {
                id: new.id.clone(),
                of: ids[0].clone(),
                duplicate: false,
                distinct: true,
            }),
            true,
        )
        .unwrap();
        assert!(crate::intake::open_hint_pairs(&b.snapshot(None).unwrap()).is_empty());
        let after = b.snapshot(None).unwrap().get(&new.id).unwrap().clone();
        assert!(rehint(&mut b, &ctx(), &after).unwrap().is_empty());
    }

    #[test]
    fn resolving_as_duplicate_closes_with_a_reason() {
        let (mut b, ids) = board(&[OLD, NEW]);
        dupes(
            &mut b,
            &ctx(),
            Some(&DupesCommand::Resolve {
                id: ids[1].clone(),
                of: ids[0].clone(),
                duplicate: true,
                distinct: false,
            }),
            true,
        )
        .unwrap();
        let closed = b.snapshot(None).unwrap().get(&ids[1]).unwrap().clone();
        assert_eq!(closed.status, "closed");
        assert_eq!(closed.outcome.as_deref(), Some("superseded"));
        assert!(closed.close_reason.unwrap().contains(&ids[0]));
    }

    #[test]
    fn a_distinct_from_pair_is_never_hinted() {
        let (b, ids) = board(&[OLD]);
        let (req, (hints, _)) = hinted(&b, std::slice::from_ref(&ids[0]), false).unwrap();
        assert!(hints.is_empty(), "{hints:?}");
        assert!(req.extra.contains(&(
            crate::intake::DISTINCT_FROM.into(),
            Obj::Iri(vocab::item_iri(&ids[0]))
        )));
    }

    #[test]
    fn an_inbox_pointer_is_never_a_candidate() {
        let (b, _) = board(&[&format!("inbox: {OLD}")]);
        let mut req = CreateReq {
            title: format!("inbox: {NEW}"),
            ..Default::default()
        };
        let (hints, _) = admit(&b, &mut req, &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert!(hints.is_empty(), "{hints:?}");
    }

    #[test]
    fn an_inbox_ephemeral_or_machine_create_is_never_hinted() {
        let (inner, ids) = board(&[OLD]);
        // The remote resolver readily matches a pointer to the item it names
        // (0.95: under the 0.99 refusal gate, over the 0.90 hint floor).
        let remote = Remote {
            name_score: Some(0.95),
            inner,
            candidate: vocab::item_iri(&ids[0]),
            fail: false,
            closed_over_cap: false,
        };
        let cases = [
            CreateReq {
                title: format!("inbox: {NEW}"),
                ..Default::default()
            },
            CreateReq {
                title: NEW.into(),
                labels: vec!["inbox".into()],
                ..Default::default()
            },
            CreateReq {
                title: NEW.into(),
                ephemeral: true,
                ..Default::default()
            },
            CreateReq {
                title: NEW.into(),
                issue_type: Some("ops".into()),
                ..Default::default()
            },
            CreateReq {
                title: NEW.into(),
                issue_type: Some("ask".into()),
                ..Default::default()
            },
        ];
        for case in cases {
            let mut req = CreateReq {
                description: Some("same underlying problem".into()),
                ..case
            };
            let (hints, unknown) = admit(&remote, &mut req, &[], None, Some(&mode(&[], false)))
                .unwrap()
                .unwrap();
            assert!(
                hints.is_empty() && unknown.is_none(),
                "{}: {hints:?}",
                req.title
            );
            assert!(
                !req.extra
                    .iter()
                    .any(|(p, _)| crate::intake::is_hint_predicate(p)),
                "{}: {:?}",
                req.title,
                req.extra
            );
            let mut strict = req.clone();
            strict.extra.clear();
            assert!(
                admit(&remote, &mut strict, &[], None, Some(&mode(&[], true))).is_ok(),
                "--strict refuses no hint on {}",
                req.title
            );
        }
        // Without the filter the same remote match is a hint.
        let mut req = CreateReq {
            title: NEW.into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let (hints, _) = admit(&remote, &mut req, &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert_eq!(hints.len(), 1, "{hints:?}");
    }

    #[test]
    fn sd_dupes_never_lists_an_ineligible_hinted_side() {
        let (mut b, ids) = board(&[OLD]);
        // A hint recorded before the filter, on an ephemeral pointer.
        let mut req = CreateReq {
            title: format!("inbox: {NEW}"),
            ..Default::default()
        };
        req.extra.extend(crate::intake::hint_facts(
            &[Hint {
                id: ids[0].clone(),
                title: OLD.into(),
                status: "open".into(),
                score: 0.97,
                matched_on: "embedding:0.97".into(),
            }],
            None,
        ));
        crate::engine::create(&mut b, &ctx(), &req).unwrap();
        assert!(crate::intake::open_hint_pairs(&b.snapshot(None).unwrap()).is_empty());
    }

    #[test]
    fn strict_refuses_at_the_threshold() {
        let (b, ids) = board(&[OLD]);
        let error = hinted(&b, &[], true).unwrap_err();
        assert!(error.message.contains("--strict") && error.message.contains(&ids[0]));
        assert_eq!(error.exit_code(), 5, "--strict is a refusal (exit 5)");
        assert!(hinted(&b, std::slice::from_ref(&ids[0]), true).is_ok());
    }

    #[test]
    fn a_resolver_timeout_is_unknown_and_the_create_proceeds_with_a_warning() {
        let (inner, _) = board(&[]);
        let remote = Remote {
            name_score: None,
            inner,
            candidate: String::new(),
            fail: true,
            closed_over_cap: false,
        };
        let mut req = CreateReq {
            title: "Requests hang under load".into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let (hints, unknown) = admit(&remote, &mut req, &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert!(hints.is_empty());
        assert!(unknown.as_deref().unwrap().contains("unavailable"));
        assert!(req
            .extra
            .contains(&(crate::intake::hint_state(), Obj::Str("unknown".into()))));
        let lines = crate::intake::hint_lines(
            &hints,
            unknown.as_deref(),
            crate::intake::HintContext::Create,
        );
        assert!(lines[0].contains("UNKNOWN") && lines[0].contains("refusal gate was not applied"));
        assert!(!lines[0].contains("not affected"), "{lines:?}");
        let edit =
            crate::intake::hint_lines(&hints, unknown.as_deref(), crate::intake::HintContext::Edit);
        assert!(
            edit[0].contains("UNKNOWN") && !edit[0].contains("gate"),
            "{edit:?}"
        );
        let mut strict = req.clone();
        assert!(admit(&remote, &mut strict, &[], None, Some(&mode(&[], true))).is_err());
    }

    #[test]
    fn a_closed_corpus_over_the_cap_is_unknown_and_the_create_proceeds() {
        // aegis-n0wc: ~380 seeds closed in 30 days were read seed by seed and
        // cost every create ~15 s. Over the cap that half is skipped, the hint
        // is UNKNOWN with a warning, and only --strict refuses.
        let (inner, _) = board(&[]);
        let remote = Remote {
            name_score: None,
            inner,
            candidate: String::new(),
            fail: false,
            closed_over_cap: true,
        };
        let mut req = CreateReq {
            title: "Requests hang under load".into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let (hints, unknown) = admit(&remote, &mut req, &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert!(hints.is_empty());
        let why = unknown.as_deref().unwrap();
        assert!(why.contains("recently closed seeds not checked"), "{why}");
        assert!(
            why.contains(&crate::intake::HINT_CLOSED_CAP.to_string()),
            "{why}"
        );
        // The resolver ran, so the line must not claim the gate was skipped.
        let lines = crate::intake::hint_lines(
            &hints,
            unknown.as_deref(),
            crate::intake::HintContext::Create,
        );
        assert!(
            lines[0].contains("PARTIAL") && lines[0].contains("refusal gate ran in full"),
            "{lines:?}"
        );
        assert!(!lines[0].contains("not applied"), "{lines:?}");
        assert!(req
            .extra
            .contains(&(crate::intake::hint_state(), Obj::Str("unknown".into()))));
        let mut strict = req.clone();
        assert!(admit(&remote, &mut strict, &[], None, Some(&mode(&[], true))).is_err());
        // Under the cap nothing changes: the same create is not UNKNOWN.
        let (inner, _) = board(&[]);
        let under = Remote {
            closed_over_cap: false,
            inner,
            ..remote
        };
        let mut req = CreateReq {
            title: "Requests hang under load".into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let (_, unknown) = admit(&under, &mut req, &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert!(unknown.is_none(), "{unknown:?}");
    }

    #[test]
    fn a_remote_name_match_below_the_hint_floor_is_not_a_hint() {
        let (inner, ids) = board(&["Requests hang under load"]);
        let req = || CreateReq {
            title: "Rotate the backup keys".into(),
            description: Some("same underlying problem".into()),
            ..Default::default()
        };
        let mut remote = Remote {
            inner,
            candidate: vocab::item_iri(&ids[0]),
            name_score: Some(0.80),
            fail: false,
            closed_over_cap: false,
        };
        let (hints, _) = admit(&remote, &mut req(), &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert!(
            hints.is_empty(),
            "a 0.80 remote name match is noise: {hints:?}"
        );
        remote.name_score = Some(0.95);
        let (hints, _) = admit(&remote, &mut req(), &[], None, Some(&mode(&[], false)))
            .unwrap()
            .unwrap();
        assert_eq!(hints.len(), 1, "{hints:?}");
    }

    #[test]
    fn distinct_from_must_name_an_existing_seed() {
        let (b, _) = board(&[OLD]);
        assert!(hinted(&b, &["fixture-nope".into()], false).is_err());
    }
}
