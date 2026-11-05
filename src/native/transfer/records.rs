//! Storage-independent planning, exact external identities and atomic imports.
use super::{conflict, Adapter, Item, Note, Remote};
use crate::{
    backend::{Backend, Ctx, SeedWrite, WriteBatch},
    error::{Result, SdError},
    model::{Comment, Seed},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn from_seed(s: &Seed, comments: &[Comment]) -> Item {
    Item {
        title: s.title.clone(),
        description: s.description.clone().unwrap_or_default(),
        status: s.status.clone(),
        priority: s.priority,
        issue_type: s.issue_type.clone(),
        assignee: s.assignee.clone(),
        labels: s.labels.iter().cloned().collect(),
        parent: s.parent.clone(),
        blocked_on: s.blocked_on.iter().cloned().collect(),
        comments: comments
            .iter()
            .filter(|c| c.seed == s.id)
            .map(|c| Note {
                key: format!("{}:{}", s.id, c.index),
                author: c.author.clone(),
                at: c.created_at.clone(),
                text: c.text.clone(),
            })
            .collect(),
    }
}
fn id(prefix: &str, link: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(link.as_bytes()));
    format!("{prefix}-ext-{}", &digest[..32])
}

fn validate(i: &Item) -> Result<()> {
    if i.title.trim().is_empty()
        || i.priority > 4
        || !matches!(
            i.status.as_str(),
            "open" | "in_progress" | "blocked" | "closed"
        )
    {
        return Err(SdError::refused(
            "transfer requires a title, priority 0..4 and open/in_progress/blocked/closed state",
        ));
    }
    if i.description.contains("<!-- seeds-origin:")
        || i.description.contains("\n\n## Seeds dependencies\n")
    {
        return Err(SdError::refused(
            "description contains reserved transfer markers; reconcile before transfer",
        ));
    }
    crate::model::parse_type(&i.issue_type)?;
    Ok(())
}

pub(super) fn export(
    b: &mut dyn Backend,
    ctx: &Ctx,
    graph: &str,
    adapter: &dyn Adapter,
    ids: &[String],
    execute: bool,
    overwrite: bool,
) -> Result<Vec<Value>> {
    let snap = b.snapshot_items(ids)?;
    let mut plans = Vec::new();
    // Resolve every origin before the first write; ambiguity never chooses a side.
    for seed_id in ids {
        let seed = snap.get(seed_id)?;
        if seed.ephemeral {
            return Err(SdError::refused("ephemeral seeds cannot be exported"));
        }
        let item = from_seed(seed, &snap.comments);
        validate(&item)?;
        let origin = format!("{graph}#{seed_id}");
        let current = if let Some(link) = &seed.external_ref {
            let r = adapter.get_link(link)?;
            if &r.link != link {
                return Err(conflict(
                    "external reference does not match configured target",
                ));
            }
            Some(r)
        } else {
            adapter.find(&origin)?
        };
        if let (Some(link), Some(remote)) = (&seed.external_ref, &current) {
            if link != &remote.link {
                return Err(conflict(
                    "seed has a different external reference; preserve it",
                ));
            }
        }
        if seed.external_ref.is_some() && current.is_none() {
            return Err(conflict(
                "linked external item missing or does not prove its origin",
            ));
        }
        plans.push((seed.clone(), origin, item, current, false));
    }
    for (_, _, item, _, _) in &plans {
        if item
            .parent
            .iter()
            .chain(item.blocked_on.iter())
            .any(|id| !ids.contains(id))
        {
            return Err(SdError::refused(
                "select all parent/dependency targets in the same export",
            ));
        }
    }
    let known_links: BTreeMap<_, _> = plans
        .iter()
        .filter_map(|(s, _, _, r, _)| r.as_ref().map(|r| (s.id.clone(), r.link.clone())))
        .collect();
    for (_, _, item, current, differing) in &mut plans {
        if let Some(remote) = current {
            let mut expected = item.clone();
            expected.parent = expected
                .parent
                .map(|id| known_links.get(&id).cloned().unwrap_or(id));
            expected.blocked_on = expected
                .blocked_on
                .into_iter()
                .map(|id| known_links.get(&id).cloned().unwrap_or(id))
                .collect();
            expected.blocked_on.sort();
            expected.labels.sort();
            let mut actual = remote.item.clone();
            actual.blocked_on.sort();
            actual.labels.sort();
            *differing = actual != expected;
            if execute && *differing && !overwrite {
                return Err(conflict("linked external fields differ; preview and explicitly choose --overwrite or reconcile"));
            }
        }
    }
    let mut out = Vec::new();
    let mut targets = BTreeMap::new();
    let mut relationships = Vec::new();
    for (seed, origin, item, current, differing) in plans {
        let action = if current.is_none() {
            "create"
        } else if differing {
            "update"
        } else {
            "unchanged"
        };
        let mut link = current.as_ref().map(|r| r.link.clone());
        if execute {
            // A create may have landed before an ambiguous response. Its origin
            // marker is found on the next invocation before another create.
            let remote = if action == "unchanged" {
                current.clone().unwrap()
            } else {
                adapter.put(&origin, &item, current.as_ref())?
            };
            link = Some(remote.link.clone());
            if seed.external_ref.as_ref() != Some(&remote.link) {
                crate::engine::update(
                    b,
                    ctx,
                    std::slice::from_ref(&seed.id),
                    &crate::engine::UpdateReq {
                        external_ref: Some(remote.link.clone()),
                        if_revision: Some(seed.revision),
                        ..Default::default()
                    },
                )?;
            }
            targets.insert(seed.id.clone(), remote.clone());
            relationships.push((remote, item.clone(), origin));
        }
        out.push(json!({"seed":seed.id,"action":action,"external_ref":link,"conflict":differing&&!overwrite,"fields":item}));
    }
    if execute {
        for (remote, item, origin) in relationships {
            adapter.relate(&remote, &item, &targets, &origin)?;
        }
    }
    Ok(out)
}

pub(super) fn import(
    b: &mut dyn Backend,
    ctx: &Ctx,
    graph: &str,
    adapter: &dyn Adapter,
    ids: &[String],
    execute: bool,
    overwrite: bool,
) -> Result<Vec<Value>> {
    let remotes = ids
        .iter()
        .map(|id| adapter.get(id))
        .collect::<Result<Vec<Remote>>>()?;
    let hashed_ids = remotes
        .iter()
        .map(|r| id(&ctx.prefix, &r.link))
        .collect::<Vec<_>>();
    let candidates = remotes
        .iter()
        .map(|r| {
            r.origin
                .as_deref()
                .and_then(|o| o.strip_prefix(&format!("{graph}#")))
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    let mut reads = hashed_ids.clone();
    reads.extend(candidates.iter().flatten().cloned());
    let snap = b.snapshot_items(&reads)?;
    let local_ids = remotes
        .iter()
        .zip(hashed_ids)
        .zip(candidates)
        .map(|((r, hashed), candidate)| {
            candidate
                .filter(|id| {
                    snap.seeds
                        .get(id)
                        .is_some_and(|s| s.external_ref.as_deref() == Some(&r.link))
                })
                .unwrap_or(hashed)
        })
        .collect::<Vec<_>>();
    let links: BTreeMap<_, _> = remotes
        .iter()
        .zip(&local_ids)
        .map(|(r, id)| (r.link.clone(), id.clone()))
        .collect();
    let mut batch = WriteBatch {
        source: "seeds:task-manager-import".into(),
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut post_state = Vec::new();
    for (r, seed_id) in remotes.into_iter().zip(local_ids) {
        validate(&r.item)?;
        let old = snap.seeds.get(&seed_id);
        if old.is_some_and(|s| s.external_ref.as_deref() != Some(&r.link)) {
            return Err(conflict("external identity collides with an existing seed"));
        }
        let mut s = old.cloned().unwrap_or_else(|| Seed {
            id: seed_id.clone(),
            created_at: ctx.now.clone(),
            created_by: Some(ctx.actor.clone()),
            ..Default::default()
        });
        let resolve = |link: &String| {
            links.get(link).cloned().ok_or_else(|| {
                SdError::refused(
                    "select the parent and dependency targets in the same import; no dangling edge",
                )
            })
        };
        s.title = r.item.title.clone();
        s.description = (!r.item.description.is_empty()).then(|| r.item.description.clone());
        s.status = r.item.status.clone();
        s.priority = r.item.priority;
        s.issue_type = r.item.issue_type.clone();
        s.assignee = r.item.assignee.clone();
        s.labels = r.item.labels.iter().cloned().collect();
        s.parent = r.item.parent.as_ref().map(resolve).transpose()?;
        s.blocked_on = r
            .item
            .blocked_on
            .iter()
            .map(resolve)
            .collect::<Result<BTreeSet<_>>>()?;
        s.external_ref = Some(r.link.clone());
        if s.status == "closed" {
            s.closed_at = old
                .and_then(|x| x.closed_at.clone())
                .or_else(|| Some(ctx.now.clone()));
            s.outcome = old
                .and_then(|o| o.outcome.clone())
                .or_else(|| Some("done".into()));
        } else {
            s.closed_at = None;
            s.outcome = None;
            s.close_reason = None;
        }
        let changed = old.is_none_or(|o| o != &s);
        if execute && old.is_some() && changed && !overwrite {
            return Err(conflict("import conflicts with existing seed; preview then reconcile or explicitly choose --overwrite"));
        }
        let action = if old.is_none() {
            "create"
        } else if changed {
            "update"
        } else {
            "unchanged"
        };
        post_state.push(s.clone());
        if changed {
            s.updated_at = ctx.now.clone();
            s.revision = old.map_or(1, |o| o.revision + 1);
            batch.seeds.push(SeedWrite {
                seed: s,
                expected_revision: old.map(|o| o.revision),
            });
        }
        // Stable external comment keys survive text edits without duplication;
        // changed existing comments are a conflict, never silently replaced.
        let existing = snap
            .comments
            .iter()
            .filter(|c| c.seed == seed_id)
            .collect::<Vec<_>>();
        let mut index = existing.iter().map(|c| c.index).max().unwrap_or(0);
        for note in &r.item.comments {
            // Exported comments keep their original immutable local identity.
            if let Some(c) = existing
                .iter()
                .find(|c| note.key == format!("{}:{}", seed_id, c.index))
            {
                if c.author != note.author || c.created_at != note.at || c.text != note.text {
                    return Err(conflict(
                        "exported local comment changed; reconcile immutable history",
                    ));
                }
                continue;
            }
            let marker = format!("\n\n[external-comment:{}]", note.key);
            let text = format!("{} ({})\n{}{}", note.author, note.at, note.text, marker);
            match existing.iter().find(|c| c.text.ends_with(&marker)) {
                Some(c) if c.text != text => {
                    return Err(conflict(
                        "external comment changed; existing seed comment is immutable",
                    ))
                }
                Some(_) => {}
                None => {
                    index += 1;
                    batch.comments.push(Comment {
                        seed: seed_id.clone(),
                        index,
                        author: ctx.actor.clone(),
                        text,
                        created_at: ctx.now.clone(),
                        extra: BTreeSet::new(),
                    });
                }
            }
        }
        out.push(json!({"seed":seed_id,"action":action,"external_ref":r.link,"conflict":changed&&old.is_some()&&!overwrite,"fields":r.item}));
    }
    check_cycles(&post_state)?;
    if execute && !batch.is_empty() {
        b.commit(&batch, ctx)?;
    }
    Ok(out)
}
fn check_cycles(seeds: &[Seed]) -> Result<()> {
    fn walk(
        id: &str,
        edges: &BTreeMap<String, Vec<String>>,
        path: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> bool {
        if done.contains(id) {
            return false;
        }
        if !path.insert(id.into()) {
            return true;
        }
        if edges
            .get(id)
            .is_some_and(|next| next.iter().any(|n| walk(n, edges, path, done)))
        {
            return true;
        }
        path.remove(id);
        done.insert(id.into());
        false
    }
    let edges = seeds
        .iter()
        .map(|seed| {
            (
                seed.id.clone(),
                seed.blocked_on
                    .iter()
                    .chain(seed.parent.iter())
                    .cloned()
                    .collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for key in edges.keys() {
        if walk(key, &edges, &mut BTreeSet::new(), &mut BTreeSet::new()) {
            return Err(SdError::refused("cyclic imported dependencies or parents"));
        }
    }
    Ok(())
}
