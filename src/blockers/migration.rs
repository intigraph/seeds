//! One revision-checked batch for exact active legacy resume markers.
use super::{condition, load, save, Blocker, State};
use crate::backend::{Backend, Ctx, SeedQuery, SeedWrite, WriteBatch};
use crate::error::{Result, SdError};

/// Plan or execute migration. Dry-run and apply use the same validated plan.
pub fn migrate(b: &mut dyn Backend, ctx: &Ctx, execute: bool) -> Result<serde_json::Value> {
    let graph = b
        .project_graph()
        .ok_or_else(|| SdError::refused("migration requires an explicit project graph"))?
        .to_string();
    let snapshot = b.snapshot_where(&SeedQuery {
        statuses: Some(["deferred".into()].into()),
        ..Default::default()
    })?;
    let mut batch = WriteBatch {
        source: "seeds:blocker-marker-migration".into(),
        ..Default::default()
    };
    let mut plan = Vec::new();
    let mut refused = Vec::new();
    for seed in snapshot.seeds.values() {
        let notes = seed.notes.as_deref().unwrap_or("");
        let markers: Vec<_> = notes
            .lines()
            .filter_map(|line| {
                line.strip_prefix("resume_when:")
                    .and_then(|value| value.trim().strip_prefix("closed:"))
                    .filter(|id| !id.is_empty() && !id.chars().any(char::is_whitespace))
            })
            .collect();
        let active_count = notes
            .lines()
            .filter(|line| line.starts_with("resume_when:"))
            .count();
        if active_count == 0 {
            continue;
        }
        if markers.len() != 1 || active_count != 1 || seed.defer_until.is_some() {
            refused.push(serde_json::json!({"issue_id":seed.id,"reason":"multiple conditions or independent date hold"}));
            continue;
        }
        let target = markers[0];
        let targets = b.snapshot_seeds(&[target.into()])?;
        if targets.get(target).is_err() {
            refused
                .push(serde_json::json!({"issue_id":seed.id,"reason":"target coverage unknown"}));
            continue;
        }
        let mut ledger = load(seed)?;
        let iri = crate::vocab::item_iri(target);
        if ledger.blockers.contains_key(&iri) {
            refused
                .push(serde_json::json!({"issue_id":seed.id,"reason":"blocker already declared"}));
            continue;
        }
        let mut blocker = Blocker {
            iri: iri.clone(),
            graph: Some(graph.clone()),
            ask: condition::ask_for(&iri, "closed")?,
            condition: "closed".into(),
            state: State::Unknown,
            observed_at: Some(ctx.now.clone()),
            actor: ctx.actor.clone(),
            created_at: ctx.now.clone(),
        };
        blocker.state = condition::observe(b, &blocker, None);
        ledger.blockers.insert(iri.clone(), blocker);
        ledger.managed_status = true;
        let mut after = seed.clone();
        after.status = "blocked".into();
        let retained = notes
            .lines()
            .filter(|line| !line.starts_with("resume_when:"))
            .collect::<Vec<_>>()
            .join("\n");
        after.notes = (!retained.is_empty()).then_some(retained);
        after.updated_at = ctx.now.clone();
        after.revision = seed
            .revision
            .checked_add(1)
            .ok_or_else(|| SdError::failed("revision overflow"))?;
        save(&mut after, &ledger)?;
        batch.seeds.push(SeedWrite {
            seed: after,
            expected_revision: Some(seed.revision),
        });
        plan.push(serde_json::json!({"issue_id":seed.id,"revision":seed.revision,"blocker":iri,"graph":graph}));
    }
    if execute && !refused.is_empty() {
        return Err(SdError::refused(
            "migration has unresolved rows; no batch written; inspect --dry-run",
        ));
    }
    let tx = if execute && !batch.is_empty() {
        Some(b.commit(&batch, ctx)?)
    } else {
        None
    };
    Ok(
        serde_json::json!({"dry_run":!execute,"count":plan.len(),"plan":plan,"refused":refused,"tx":tx}),
    )
}

/// The latest ISO date (`YYYY-MM-DD`) a free-text note names, if any.
fn latest_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut best: Option<String> = None;
    for start in 0..bytes.len().saturating_sub(9) {
        let window = &bytes[start..start + 10];
        let shaped = window.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        });
        let bounded = start == 0 || !bytes[start - 1].is_ascii_digit();
        let after = bytes.get(start + 10).is_none_or(|c| !c.is_ascii_digit());
        if !(shaped && bounded && after) {
            continue;
        }
        let Ok(date) = super::recheck_instant(std::str::from_utf8(window).unwrap_or("")) else {
            continue;
        };
        if best.as_deref().is_none_or(|b| date.as_str() > b) {
            best = Some(date);
        }
    }
    best
}

/// Plan or execute the one-time migration of `blocked:*` labels into typed
/// blocks (aegis-xjh). A seed with an open dependency keeps that edge as its
/// condition; any other becomes a gate rechecked at the latest date its notes
/// name, or at the migration instant when they name none (so it is lapsed at
/// once and returns to ready). Every row is planned and printed; nothing is
/// dropped silently. `blocked:human` stays as the ASK visibility class.
pub fn migrate_labels(b: &mut dyn Backend, ctx: &Ctx, execute: bool) -> Result<serde_json::Value> {
    use super::{gate_lapsed, Gate, BLOCK_LABEL_PREFIX};
    let snap = b.snapshot(None)?;
    let mut batch = WriteBatch {
        source: "seeds:blocked-label-migration".into(),
        ..Default::default()
    };
    let (mut plan, mut refused) = (Vec::new(), Vec::new());
    let (mut dependency, mut from_notes, mut from_migration, mut lapsed) = (0, 0, 0, 0);
    for seed in snap.seeds.values() {
        let labels: Vec<String> = seed
            .labels
            .iter()
            .filter(|l| l.starts_with(BLOCK_LABEL_PREFIX))
            .cloned()
            .collect();
        if labels.is_empty() || seed.status == "closed" || seed.is_tombstone() {
            continue;
        }
        let mut ledger = match load(seed) {
            Ok(ledger) => ledger,
            Err(e) => {
                refused.push(serde_json::json!({"issue_id":seed.id,"reason":e.to_string()}));
                continue;
            }
        };
        let mut after = seed.clone();
        after
            .labels
            .retain(|l| !l.starts_with(BLOCK_LABEL_PREFIX) || l == "blocked:human");
        let open_deps = snap.open_blockers(seed);
        let row = if !open_deps.is_empty() {
            dependency += 1;
            serde_json::json!({"issue_id":seed.id,"revision":seed.revision,"labels":labels,
                "kind":"dependency","blocked_on":open_deps})
        } else if !ledger.gates.is_empty() {
            serde_json::json!({"issue_id":seed.id,"revision":seed.revision,"labels":labels,
                "kind":"already_gated"})
        } else {
            let (recheck_at, source) = match latest_date(seed.notes.as_deref().unwrap_or("")) {
                Some(d) => {
                    from_notes += 1;
                    (d, "notes")
                }
                None => {
                    from_migration += 1;
                    (ctx.now.clone(), "migration_date")
                }
            };
            let who = labels
                .iter()
                .map(|l| &l[BLOCK_LABEL_PREFIX.len()..])
                .collect::<Vec<_>>()
                .join(",");
            let gate = Gate {
                who: if who.trim().is_empty() {
                    "unknown".into()
                } else {
                    who
                },
                reason: format!("migrated from label {}; see notes", labels.join(", ")),
                recheck_at: super::recheck_instant(&recheck_at)?,
                actor: ctx.actor.clone(),
                created_at: ctx.now.clone(),
                lapse_generation: None,
            };
            let is_lapsed = gate_lapsed(&gate, &ctx.now);
            lapsed += usize::from(is_lapsed);
            let row = serde_json::json!({"issue_id":seed.id,"revision":seed.revision,"labels":labels,
                "kind":"gate","who":gate.who,"recheck_at":gate.recheck_at,"recheck_source":source,
                "lapsed":is_lapsed});
            ledger.gates.push(gate);
            if after.status == "open" {
                after.status = "blocked".into();
                ledger.managed_status = true;
            }
            row
        };
        if !ledger.gates.is_empty() {
            save(&mut after, &ledger)?;
        }
        after.updated_at = ctx.now.clone();
        after.revision = seed
            .revision
            .checked_add(1)
            .ok_or_else(|| SdError::failed("revision overflow"))?;
        batch.seeds.push(SeedWrite {
            seed: after,
            expected_revision: Some(seed.revision),
        });
        plan.push(row);
    }
    if execute && !refused.is_empty() {
        return Err(SdError::refused(
            "label migration has unresolved rows; no batch written; inspect --dry-run",
        ));
    }
    let tx = if execute && !batch.is_empty() {
        Some(b.commit(&batch, ctx)?)
    } else {
        None
    };
    Ok(serde_json::json!({"dry_run":!execute,"count":plan.len(),
        "summary":{"dependency":dependency,"gate_from_notes":from_notes,
            "gate_from_migration_date":from_migration,"lapsed":lapsed},
        "plan":plan,"refused":refused,"tx":tx}))
}
