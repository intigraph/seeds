//! Body-free seed reads for the duplicate intake gate (aegis-10m).
//!
//! `sd create` read every fact of every open seed to match titles: ~5 MB per
//! create on the live board. Measured 2026-11-27 over the 449 active seeds,
//! notes were 43% of the literal bytes, bridge JSON 30% and descriptions 17%;
//! the gate reads none of them. The same subjects are discovered the same way
//! and every other fact is read unchanged, so identity, status, aliases,
//! labels, dates and recorded distinctions decode exactly as before.
use super::*;

/// The long text bodies a brief read leaves out.
fn body_predicates() -> [String; 5] {
    [
        term::notes(),
        term::description(),
        term::design(),
        term::acceptance_criteria(),
        vocab::seeds("beadsJson"),
    ]
}

/// [`subjects_query`] with the body predicates filtered out in every branch.
pub(super) fn brief_subjects_query(graph: &str, subjects: &[&String]) -> String {
    let omit = body_predicates()
        .iter()
        .map(|p| format!("<{p}>"))
        .collect::<Vec<_>>()
        .join(", ");
    let branches: Vec<String> = subjects
        .iter()
        .map(|s| {
            format!(
                "{{ BIND(<{s}> AS ?s) GRAPH <{graph}> {{ <{s}> ?p ?o FILTER(?p NOT IN ({omit})) }} }}"
            )
        })
        .collect();
    format!(
        "SELECT ?s ?p ?o WHERE {{ {} }} LIMIT {ROW_CAP}",
        branches.join(" UNION ")
    )
}

impl RemoteBackend {
    /// [`Backend::snapshot_where`] without seed bodies: the same discovery,
    /// then brief reads of exactly those subjects in both graphs.
    pub(super) fn brief_snapshot_where(&self, q: &crate::backend::SeedQuery) -> Result<Snapshot> {
        if vocab::seed_query_pattern(q).is_none() {
            return self.snapshot(None);
        }
        let mut subjects = BTreeSet::new();
        for graph in self.both_graphs() {
            let pattern = self
                .owner_seed_query_pattern(&graph, q)?
                .expect("constrained seed query retains a pattern");
            subjects.extend(self.subjects_where(&graph, &pattern, None)?);
        }
        let all: Vec<&String> = subjects.iter().collect();
        let mut main = Facts::new();
        let mut eph = Facts::new();
        let eph_graph = vocab::ephemeral_graph(&self.graph);
        let read_eph = !subjects.is_empty() && self.ephemeral_may_hold_facts()?;
        for chunk in all.chunks(BULK_FACT_SUBJECTS_PER_READ) {
            self.read_subjects_shaped(&self.graph, chunk, None, &mut main, brief_subjects_query)?;
            if read_eph {
                self.read_subjects_shaped(&eph_graph, chunk, None, &mut eph, brief_subjects_query)?;
            }
        }
        Ok(Snapshot::from_graphs(&main, &eph))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brief_read_omits_exactly_the_bodies_in_every_branch() {
        let a = "https://seeds.local/item/x-1".to_string();
        let b = "https://seeds.local/item/x-2".to_string();
        let q = brief_subjects_query("urn:g", &[&a, &b]);
        assert_eq!(q.matches("FILTER(?p NOT IN (").count(), 2);
        for p in body_predicates() {
            assert_eq!(q.matches(&format!("<{p}>")).count(), 2, "{p}");
        }
        for kept in [term::status(), term::name(), vocab::RDFS_LABEL.to_string()] {
            assert!(!q.contains(&kept), "{kept} must still be read");
        }
        assert!(q.ends_with(&format!("LIMIT {ROW_CAP}")));
    }
}
