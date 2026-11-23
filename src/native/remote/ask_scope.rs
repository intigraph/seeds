//! Staged native-settler mode: audit and item changes share ONE graph/CAS.
//! Ordinary CLI writes retain the project#provenance layout. Activation of
//! this storage choice needs the operator's separate storage-authority ruling.
use super::RemoteBackend;
#[cfg(unix)]
use crate::error::{Result, SdError};
#[cfg(unix)]
use crate::model::Obj;
use crate::vocab;
#[cfg(unix)]
use std::collections::{BTreeMap, BTreeSet};

impl RemoteBackend {
    #[cfg(unix)]
    pub(crate) fn with_project_settler_audit(mut self) -> Self {
        self.project_provenance = true;
        self
    }

    pub(super) fn write_record_graph(&self) -> String {
        if self.project_provenance {
            self.graph.clone()
        } else {
            vocab::provenance_graph(&self.graph)
        }
    }

    /// Bounded reader for the audit location used only by native settlement.
    /// A unique writer and complete rows are required; not a recent-only view.
    #[cfg(unix)]
    pub(crate) fn settler_audit_for(
        &self,
        id: &str,
        revision: u64,
    ) -> Result<Vec<BTreeMap<String, Obj>>> {
        if id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(SdError::refused("invalid settler audit item"));
        }
        let query = format!(
            "SELECT ?write ?p ?o WHERE {{ {{ SELECT ?write WHERE {{ GRAPH <{}> {{ ?write <{}> <{}> ; <{}> \"seeds:ask-settlement\" ; <{}> \"{}@{}\" }} }} LIMIT 2 }} GRAPH <{}> {{ ?write ?p ?o }} }} LIMIT 65",
            self.graph, vocab::seeds("wrote"), vocab::item_iri(id), vocab::seeds("source"), vocab::seeds("version"), id, revision, self.graph
        );
        let (rows, truncated) = self.select_marked(&query, None)?;
        let writers: BTreeSet<_> = rows.iter().filter_map(|row| row.get("write")).collect();
        if truncated || rows.len() >= 65 || writers.len() != 1 {
            return Err(SdError::failed(
                "native settler audit is missing, ambiguous or incomplete; reconcile only",
            ));
        }
        Ok(rows)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::backend::{Backend, Ctx, SeedWrite, WriteBatch};
    use crate::engine::{self, CreateReq};

    #[test]
    fn project_settler_audit_is_readable_by_exact_item_and_revision() {
        let Some(server) = super::super::tests::quipu_server() else {
            return;
        };
        let graph = "urn:test:native-settler-audit";
        let ctx = Ctx {
            now: "2026-11-23T00:00:00Z".into(),
            actor: "fixture-settler".into(),
            prefix: "fixture".into(),
            claims: Default::default(),
        };
        let mut ordinary = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
        assert_eq!(
            ordinary.write_record_graph(),
            vocab::provenance_graph(graph)
        );
        let seed = engine::create(
            &mut ordinary,
            &ctx,
            &CreateReq {
                title: "Audit reader control".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        let mut native = RemoteBackend::connect(&server.base, graph, None, None, &[])
            .unwrap()
            .with_project_settler_audit();
        assert_eq!(native.write_record_graph(), graph);
        assert!(native.settler_audit_for(&seed.id, seed.revision).is_err());
        let mut next = seed.clone();
        next.revision += 1;
        native
            .commit(
                &WriteBatch {
                    seeds: vec![SeedWrite {
                        seed: next.clone(),
                        expected_revision: Some(seed.revision),
                    }],
                    source: "seeds:ask-settlement".into(),
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        let rows = native.settler_audit_for(&seed.id, next.revision).unwrap();
        assert!(rows
            .iter()
            .any(|row| row.get("p") == Some(&Obj::Iri(vocab::seeds("actor")))
                && row.get("o") == Some(&Obj::Str(ctx.actor.clone()))));
        assert!(native
            .settler_audit_for(&seed.id, next.revision + 1)
            .is_err());
        assert_eq!(
            ordinary.write_record_graph(),
            vocab::provenance_graph(graph)
        );
    }
}
