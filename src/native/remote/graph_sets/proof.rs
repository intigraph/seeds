//! One authenticated composed answer proves post-state AND exact provenance.
use super::super::{batch_subjects, decode_select_general, decode_select_typed, Facts, ROW_CAP};
use super::*;
use crate::backend::{SeedWrite, WriteBatch};
use crate::model::{Comment, Obj, Seed, Snapshot};
impl Mode {
    pub(super) fn prove(&self, remote: &RemoteBackend, a: &Attempt, tx: u64) -> Result<String> {
        let expected_seeds: Vec<Seed> = a
            .expected
            .seeds
            .iter()
            .map(|f| Seed::from_facts(f).ok_or_else(|| invalid("expected seed facts")))
            .collect::<Result<_>>()?;
        let expected_comments: Vec<Comment> = a
            .expected
            .comments
            .iter()
            .map(|f| Comment::from_facts(f).ok_or_else(|| invalid("expected comment facts")))
            .collect::<Result<_>>()?;
        let batch = WriteBatch {
            seeds: expected_seeds
                .iter()
                .cloned()
                .map(|seed| SeedWrite {
                    seed,
                    expected_revision: None,
                })
                .collect(),
            comments: expected_comments.clone(),
            delete_seeds: a.expected.delete_seeds.clone(),
            delete_comments: a.expected.delete_comments.clone(),
            source: String::new(),
        };
        let mut subjects = batch_subjects(&batch);
        subjects.extend(a.expected.record.iter().map(|(s, _)| s.clone()));
        if subjects.is_empty() || a.expected.record.is_empty() {
            return Err(invalid("no complete intended-state witness"));
        }
        // Bound patterns, never a board scan; no paging across different cuts.
        let mut branches = Vec::new();
        for graph in self.profile.members() {
            for subject in &subjects {
                if !safe_iri(subject) || !safe_iri(&graph) {
                    return Err(invalid("proof IRI"));
                }
                branches.push(format!("{{ GRAPH <{graph}> {{ <{subject}> ?p ?o }} BIND(<{graph}> AS ?g) BIND(<{subject}> AS ?s) }}"));
            }
        }
        let query = format!(
            "SELECT ?g ?s ?p ?o WHERE {{ {} }} LIMIT {ROW_CAP}",
            branches.join(" UNION ")
        );
        let (body, cut) =
            self.query_with_cut(remote, &json!({"query":query}).to_string(), Some(tx))?;
        let (rows, truncated) = match decode_select_typed(&body) {
            Some(v) => v,
            None => decode_select_general(&body)?,
        };
        if truncated || rows.len() >= ROW_CAP {
            return Err(invalid("proof answer incomplete"));
        }
        let mut main = Facts::new();
        let mut ephemeral = Facts::new();
        let mut record = Facts::new();
        for mut row in rows {
            let (Some(Obj::Iri(g)), Some(Obj::Iri(s)), Some(Obj::Iri(p)), Some(o)) = (
                row.remove("g"),
                row.remove("s"),
                row.remove("p"),
                row.remove("o"),
            ) else {
                return Err(invalid("proof row"));
            };
            if !subjects.contains(&s) {
                return Err(invalid("proof returned unrequested subject"));
            }
            let out = if g == self.profile.project {
                &mut main
            } else if g == crate::vocab::ephemeral_graph(&self.profile.project) {
                &mut ephemeral
            } else if g == super::super::provenance_graph(&self.profile.project) {
                &mut record
            } else {
                return Err(invalid("proof graph"));
            };
            out.entry(s).or_default().push((p, o));
        }
        let after = Snapshot::from_graphs(&main, &ephemeral);
        if expected_seeds
            .iter()
            .any(|s| after.seeds.get(&s.id) != Some(s))
            || expected_comments
                .iter()
                .any(|c| !after.comments.contains(c))
            || a.expected
                .delete_seeds
                .iter()
                .any(|(id, _)| after.seeds.contains_key(id))
            || a.expected.delete_comments.iter().any(|(id, n)| {
                after
                    .comments
                    .iter()
                    .any(|c| c.seed == *id && c.index == *n)
            })
            || a.expected.record.iter().any(|(s, expected)| {
                !record
                    .get(s)
                    .is_some_and(|facts| expected.iter().all(|f| facts.contains(f)))
            })
        {
            return Err(invalid("intended state or exact provenance not proven"));
        }
        Ok(cut)
    }
}
fn safe_iri(value: &str) -> bool {
    !value
        .chars()
        .any(|c| c.is_control() || "<>\"{}|^`\\ ".contains(c))
}
