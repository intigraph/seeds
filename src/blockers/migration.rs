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
