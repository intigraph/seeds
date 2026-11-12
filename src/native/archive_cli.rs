//! `sd archive` / `sd unarchive` (aegis-n0wc.1): the operator verbs over
//! [`remote::archive`](super::remote::archive). A plan unless `--execute`;
//! an executed run logs each batch's intent before its update and its result
//! after, and stops at the first batch that fails verification.

use std::collections::BTreeSet;
use std::io::Write;

use serde_json::{json, Value as Json};

use super::cli::{ArchiveArgs, UnarchiveArgs};
use super::remote::archive::Protect;
use super::remote::RemoteBackend;
use super::{ok, Outcome};
use crate::backend::{Backend, Ctx};
use crate::error::{Result, SdError};

const PAGE: usize = 500;

fn check_instant(s: &str) -> Result<()> {
    let b = s.as_bytes();
    let shaped = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z'
        && s.chars()
            .enumerate()
            .all(|(i, c)| [4, 7, 10, 13, 16, 19].contains(&i) || c.is_ascii_digit());
    if shaped {
        Ok(())
    } else {
        Err(SdError::usage(format!(
            "--closed-before must be an RFC 3339 UTC instant like 2026-10-28T00:00:00Z, not {s:?}"
        )))
    }
}

fn append(log: &std::path::Path, line: &Json) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| SdError::failed(format!("cannot open archive log {}: {e}", log.display())))?;
    writeln!(f, "{line}")
        .and_then(|()| f.sync_all())
        .map_err(|e| SdError::failed(format!("cannot write archive log {}: {e}", log.display())))
}

pub(super) fn archive(
    json_out: bool,
    ctx: &Ctx,
    remote: &RemoteBackend,
    a: &ArchiveArgs,
) -> Result<Outcome> {
    check_instant(&a.closed_before)?;
    if a.batch == 0 || a.batch > 200 {
        return Err(SdError::usage("--batch must be 1..=200"));
    }
    if a.execute && a.log.is_none() {
        return Err(SdError::usage(
            "--execute needs --log: every moved batch must be reversible",
        ));
    }
    let text = std::fs::read_to_string(&a.protect).map_err(|e| {
        SdError::usage(format!(
            "cannot read protect file {}: {e}",
            a.protect.display()
        ))
    })?;
    let protect = Protect::parse(&text)?;
    let protected = remote.archive_protected(&a.closed_before, &protect)?;
    let mut movable = Vec::new();
    let mut scanned = 0usize;
    let mut kept = 0usize;
    let mut cursor = String::new();
    loop {
        let page = remote.archive_candidates(&a.closed_before, &cursor, PAGE)?;
        let Some(last) = page.last().cloned() else {
            break;
        };
        for id in page {
            if a.limit > 0 && scanned >= a.limit {
                break;
            }
            scanned += 1;
            if protected.contains_key(&id) {
                kept += 1;
            } else {
                movable.push(id);
            }
        }
        if a.limit > 0 && scanned >= a.limit {
            break;
        }
        cursor = last;
    }
    let mut reasons = std::collections::BTreeMap::<&str, usize>::new();
    for r in protected.values() {
        *reasons.entry(*r).or_default() += 1;
    }
    let batches = movable.len().div_ceil(a.batch);
    if !a.execute {
        let value = json!({
            "mode": "plan", "closed_before": a.closed_before,
            "candidates_scanned": scanned, "protected_kept": kept, "movable": movable.len(),
            "batches": batches, "protected_by_reason": reasons,
            "sample": movable.iter().take(20).collect::<Vec<_>>(),
        });
        let text = format!(
            "archive plan (nothing moved): {scanned} closed seeds before {} scanned, {kept} protected and kept live, {} movable in {batches} batch(es) of {}",
            a.closed_before,
            movable.len(),
            a.batch
        );
        return Ok(ok(json_out, value, text, vec![]));
    }
    let log = a.log.as_ref().expect("checked above");
    let mut moved_seeds = 0usize;
    let mut moved_subjects = 0usize;
    for (n, chunk) in movable.chunks(a.batch).enumerate() {
        // The intent line is durable BEFORE the update is sent: an update can
        // commit and still fail verification (timeout, lost response, a fact
        // added since the pre-read), and `--from-log` must reverse it then.
        // `unarchive` moves back only what the archive actually holds, so an
        // intent for a batch that never landed restores nothing.
        append(
            log,
            &json!({"op": "archive", "phase": "intent", "batch": n, "ids": chunk}),
        )?;
        let m = remote.archive_move(ctx, chunk, true).map_err(|e| {
            SdError::failed(format!(
                "{}\nbatch {n} ids: {}\nits intent is logged; reverse it with: sd unarchive --from-log {} --execute",
                e.message,
                chunk.join(" "),
                log.display()
            ))
        })?;
        append(
            log,
            &json!({"op": "archive", "phase": "result", "batch": n, "tx": m.tx, "ids": m.ids, "subjects": m.subjects, "facts": m.facts}),
        )
        .map_err(|e| {
            SdError::failed(format!(
                "{}\nbatch {n} moved (tx {}) but its result line was not written; its intent line names ids: {}",
                e.message,
                m.tx,
                chunk.join(" ")
            ))
        })?;
        moved_seeds += m.ids.len();
        moved_subjects += m.subjects;
    }
    let value = json!({"mode": "execute", "moved_seeds": moved_seeds, "moved_subjects": moved_subjects,
                       "batches": batches, "protected_kept": kept, "log": log.display().to_string()});
    let text = format!(
        "archived {moved_seeds} seeds ({moved_subjects} subjects with comments) in {batches} batch(es); kept {kept} protected; log {}",
        log.display()
    );
    Ok(ok(json_out, value, text, vec![]))
}

fn log_ids(path: &std::path::Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| SdError::usage(format!("cannot read archive log {}: {e}", path.display())))?;
    let mut ids = BTreeSet::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let v: Json = serde_json::from_str(line)
            .map_err(|e| SdError::usage(format!("archive log line {} is not JSON: {e}", n + 1)))?;
        if v["op"] != "archive" {
            continue;
        }
        for id in v["ids"].as_array().into_iter().flatten() {
            ids.insert(
                id.as_str()
                    .ok_or_else(|| {
                        SdError::usage(format!("archive log line {} holds a non-string id", n + 1))
                    })?
                    .to_string(),
            );
        }
    }
    Ok(ids.into_iter().collect())
}

pub(super) fn unarchive(
    json_out: bool,
    ctx: &Ctx,
    remote: &RemoteBackend,
    a: &UnarchiveArgs,
) -> Result<Outcome> {
    if a.batch == 0 || a.batch > 200 {
        return Err(SdError::usage("--batch must be 1..=200"));
    }
    let mut ids: Vec<String> = a.ids.clone();
    if let Some(log) = &a.from_log {
        ids.extend(log_ids(log)?);
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(SdError::usage("name seed ids or --from-log"));
    }
    let mut held = Vec::new();
    for chunk in ids.chunks(a.batch) {
        held.extend(remote.snapshot_archived(chunk)?.seeds.into_keys());
    }
    held.sort();
    let absent = ids.len() - held.len();
    if !a.execute {
        let value = json!({"mode": "plan", "requested": ids.len(), "in_archive": held.len(), "absent": absent});
        let text = format!(
            "unarchive plan (nothing moved): {} requested, {} in the archive, {absent} not archived",
            ids.len(),
            held.len()
        );
        return Ok(ok(json_out, value, text, vec![]));
    }
    let mut restored = 0usize;
    let mut subjects = 0usize;
    for chunk in held.chunks(a.batch) {
        let m = remote.archive_move(ctx, chunk, false)?;
        restored += m.ids.len();
        subjects += m.subjects;
    }
    let value = json!({"mode": "execute", "restored_seeds": restored, "restored_subjects": subjects, "absent": absent});
    let text = format!("restored {restored} seeds ({subjects} subjects with comments) to the live board; {absent} were not archived");
    Ok(ok(json_out, value, text, vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cutoff_must_be_an_exact_utc_instant() {
        assert!(check_instant("2026-10-28T00:00:00Z").is_ok());
        for bad in [
            "2026-10-28",
            "2026-10-28T00:00:00+00:00",
            "2026-10-28T00:00:00",
            "x",
            "2026-1a-28T00:00:00Z",
        ] {
            assert!(check_instant(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_log_names_only_archive_batches() {
        let dir = std::env::temp_dir().join(format!("seeds-archive-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        std::fs::write(
            &path,
            "{\"op\":\"archive\",\"ids\":[\"b\",\"a\"]}\n\n{\"op\":\"other\",\"ids\":[\"z\"]}\n{\"op\":\"archive\",\"ids\":[\"a\",\"c\"]}\n",
        )
        .unwrap();
        assert_eq!(log_ids(&path).unwrap(), vec!["a", "b", "c"]);
        std::fs::write(&path, "not json\n").unwrap();
        assert!(log_ids(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
