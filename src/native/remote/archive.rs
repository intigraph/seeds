//! The board archive move (aegis-n0wc.1): closed seeds past a cutoff, with
//! their comments, move from the live project graph to `<graph>#seeds-archive`
//! under unchanged IRIs, and back again.
//!
//! Safety, in order of the failures it prevents:
//! - Selection keeps live every seed a reader may still need by id: a
//!   protect list (ids and labels), every target of a dependency edge from a
//!   seed that stays live, and every id named in a live `resume_when:` line.
//! - Each batch is ONE update whose branches name each item as a bound IRI.
//!   Never `VALUES` (quipu does not push it down: a whole-graph scan) and
//!   never a `BIND` of an outer variable (unbound inside the group: it matches
//!   the whole graph). A forward branch also requires the item to still be
//!   `closed`, so a seed reopened since selection is not moved.
//! - Each batch is verified: the facts read from the source graph before the
//!   move are exactly the facts in the destination after it, and none remain
//!   in the source. A mismatch stops the run; nothing later is moved.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as Json;

use super::{form_encode, is_query_timeout, update_report, Facts, RemoteBackend, ROW_CAP};
use crate::backend::Ctx;
use crate::error::{Result, SdError};
use crate::model::Obj;
use crate::vocab::{self, term};

/// What the operator protects beyond the structural rules: exact ids, exact
/// labels and label prefixes. Read from a JSON file with `items[].id`,
/// `protected_labels` and `protected_label_prefixes`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protect {
    pub ids: BTreeSet<String>,
    pub labels: BTreeSet<String>,
    pub label_prefixes: Vec<String>,
}

impl Protect {
    /// Parse the protect file. Malformed is an error: archive nothing.
    pub fn parse(text: &str) -> Result<Self> {
        let doc: Json = serde_json::from_str(text)
            .map_err(|e| SdError::usage(format!("protect file is not JSON: {e}")))?;
        let items = doc["items"]
            .as_array()
            .ok_or_else(|| SdError::usage("protect file needs an items array"))?;
        let mut ids = BTreeSet::new();
        for item in items {
            let id = item["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| SdError::usage("every protect item needs a non-empty id"))?;
            ids.insert(id.to_string());
        }
        let strings = |key: &str| -> Result<Vec<String>> {
            match &doc[key] {
                Json::Null => Ok(Vec::new()),
                Json::Array(a) => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| SdError::usage(format!("{key} holds a non-string")))
                    })
                    .collect(),
                _ => Err(SdError::usage(format!("{key} must be an array"))),
            }
        };
        Ok(Self {
            ids,
            labels: strings("protected_labels")?.into_iter().collect(),
            label_prefixes: strings("protected_label_prefixes")?,
        })
    }
}

/// One batch moved: what it moved and the transaction that did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub ids: Vec<String>,
    pub subjects: usize,
    pub facts: usize,
    pub tx: u64,
}

const DEPENDENCY_EDGES: fn() -> [String; 4] = || {
    [
        term::blocked_on(),
        term::child_of(),
        term::related_to(),
        term::discovered_from(),
    ]
};

fn dt(instant: &str) -> String {
    format!("\"{instant}\"^^<http://www.w3.org/2001/XMLSchema#dateTime>")
}

/// The ids named by `closed:<id>` in `resume_when:` lines of `notes`.
pub fn resume_ids(notes: &str) -> BTreeSet<String> {
    notes
        .lines()
        .filter_map(|l| l.trim().strip_prefix("resume_when:"))
        .flat_map(|rest| {
            rest.split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .filter_map(|tok| tok.strip_prefix("closed:"))
                .map(|id| {
                    id.trim_matches(|c: char| {
                        !(c.is_alphanumeric() || c == '-' || c == '.' || c == '_')
                    })
                })
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

impl RemoteBackend {
    fn archive_ids(&self, sparql: &str, var: &str) -> Result<BTreeSet<String>> {
        let (rows, truncated) = self.select_marked(sparql, None)?;
        if truncated || rows.len() >= ROW_CAP {
            return Err(SdError::failed(format!(
                "archive selection read was truncated ({} rows); refusing to select on a partial view",
                rows.len()
            )));
        }
        Ok(rows
            .into_iter()
            .filter_map(|mut r| match r.remove(var) {
                Some(Obj::Str(s)) => Some(s),
                Some(Obj::Iri(s)) => Some(s),
                _ => None,
            })
            .collect())
    }

    /// Every id that must stay live, with the first reason found. Seeds
    /// closed on or after `cutoff` stay live too, so their edges protect.
    pub fn archive_protected(
        &self,
        cutoff: &str,
        protect: &Protect,
    ) -> Result<BTreeMap<String, &'static str>> {
        let g = &self.graph;
        let status = term::status();
        let ident = term::identifier();
        let edges = DEPENDENCY_EDGES().map(|e| format!("<{e}>")).join("|");
        let mut out: BTreeMap<String, &'static str> = BTreeMap::new();
        for id in &protect.ids {
            out.entry(id.clone()).or_insert("protect list");
        }
        let live_targets = format!(
            "SELECT DISTINCT ?id WHERE {{ GRAPH <{g}> {{ ?x {edges} ?t . ?t <{ident}> ?id . \
             ?x <{status}> ?xs FILTER(?xs != \"closed\" && ?xs != \"tombstone\") }} }}"
        );
        for id in self.archive_ids(&live_targets, "id")? {
            out.entry(id)
                .or_insert("target of a live seed's dependency");
        }
        let recent_targets = format!(
            "SELECT DISTINCT ?id WHERE {{ GRAPH <{g}> {{ ?x {edges} ?t . ?t <{ident}> ?id . \
             ?x <{status}> \"closed\" ; <{}> ?m FILTER(?m >= {}) }} }}",
            term::updated_at(),
            dt(cutoff)
        );
        for id in self.archive_ids(&recent_targets, "id")? {
            out.entry(id)
                .or_insert("target of a recently closed seed's dependency");
        }
        let notes = format!(
            "SELECT ?no WHERE {{ GRAPH <{g}> {{ ?x <{status}> ?xs ; <{}> ?no \
             FILTER(?xs != \"closed\" && ?xs != \"tombstone\" && CONTAINS(?no, \"resume_when:\")) }} }}",
            term::notes()
        );
        for note in self.archive_ids(&notes, "no")? {
            for id in resume_ids(&note) {
                out.entry(id).or_insert("named in a live resume condition");
            }
        }
        if !protect.labels.is_empty() || !protect.label_prefixes.is_empty() {
            let mut tests: Vec<String> = protect
                .labels
                .iter()
                .map(|l| format!("?l = \"{}\"", crate::validate::escape_literal(l)))
                .collect();
            tests.extend(
                protect
                    .label_prefixes
                    .iter()
                    .map(|p| format!("STRSTARTS(?l, \"{}\")", crate::validate::escape_literal(p))),
            );
            let labelled = format!(
                "SELECT DISTINCT ?id WHERE {{ GRAPH <{g}> {{ ?s <{status}> \"closed\" ; <{ident}> ?id ; \
                 <{}> ?l FILTER({}) }} }}",
                term::label(),
                tests.join(" || ")
            );
            for id in self.archive_ids(&labelled, "id")? {
                out.entry(id).or_insert("protected label");
            }
        }
        Ok(out)
    }

    /// Up to `limit` closed seeds modified before `cutoff` whose id sorts
    /// after `after`, in id order: a cursor, so pages never repeat or skip.
    pub fn archive_candidates(
        &self,
        cutoff: &str,
        after: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let sparql = format!(
            "SELECT ?id WHERE {{ GRAPH <{}> {{ ?s <{}> \"closed\" ; <{}> ?id ; <{}> ?m \
             FILTER(?m < {} && STR(?id) > \"{}\") }} }} ORDER BY ?id LIMIT {}",
            self.graph,
            term::status(),
            term::identifier(),
            term::updated_at(),
            dt(cutoff),
            crate::validate::escape_literal(after),
            limit.min(ROW_CAP - 1)
        );
        let mut ids: Vec<String> = self.archive_ids(&sparql, "id")?.into_iter().collect();
        ids.sort();
        Ok(ids)
    }

    /// The facts of `items` and their comments in `graph`: one combined
    /// bound read, refusing a truncated or abandoned answer.
    fn family_facts(&self, graph: &str, items: &[String]) -> Result<Facts> {
        let iris: Vec<String> = items.iter().map(|id| vocab::item_iri(id)).collect();
        let refs: Vec<&String> = iris.iter().collect();
        let query = super::archive_family_query(graph, &refs);
        let (rows, truncated) = match self.select_marked(&query, None) {
            Err(e) if is_query_timeout(&e) => {
                return Err(SdError::failed(
                    "archive verification read timed out; batch state UNKNOWN",
                ))
            }
            other => other?,
        };
        if truncated || rows.len() >= ROW_CAP {
            return Err(SdError::failed(
                "archive verification read was truncated; use a smaller batch",
            ));
        }
        let mut facts = Facts::new();
        for mut r in rows {
            if let (Some(Obj::Iri(s)), Some(Obj::Iri(p)), Some(o)) =
                (r.remove("s"), r.remove("p"), r.remove("o"))
            {
                facts.entry(s).or_default().push((p, o));
            }
        }
        for v in facts.values_mut() {
            v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        }
        Ok(facts)
    }

    /// Move `ids` (and their comments) between the live and archive graphs in
    /// one update, then verify. `forward` = live -> archive, and moves only
    /// items still `closed`; backward moves whatever the archive holds.
    pub fn archive_move(&self, ctx: &Ctx, ids: &[String], forward: bool) -> Result<Moved> {
        if ids.is_empty() {
            return Ok(Moved {
                ids: Vec::new(),
                subjects: 0,
                facts: 0,
                tx: 0,
            });
        }
        let live = self.graph.clone();
        let arch = vocab::archive_graph(&live);
        let (src, dst) = if forward {
            (&live, &arch)
        } else {
            (&arch, &live)
        };
        let before = self.family_facts(src, ids)?;
        let status = term::status();
        // Forward moves only items the pre-read shows closed: one reopened
        // since selection is skipped (its guarded branch would match nothing).
        let found: BTreeSet<String> = ids
            .iter()
            .filter(|id| {
                before.get(&vocab::item_iri(id)).is_some_and(|facts| {
                    !forward
                        || facts
                            .iter()
                            .any(|(p, o)| *p == status && matches!(o, Obj::Str(s) if s == "closed"))
                })
            })
            .cloned()
            .collect();
        let parent = term::comment_on();
        let guard = |item: &str| {
            if forward {
                format!("<{item}> <{status}> \"closed\" . ")
            } else {
                String::new()
            }
        };
        let branches: Vec<String> = found
            .iter()
            .map(|id| vocab::item_iri(id))
            .flat_map(|item| {
                [
                    format!(
                        "{{ GRAPH <{src}> {{ {}<{item}> ?p ?o . BIND(<{item}> AS ?s) }} }}",
                        guard(&item)
                    ),
                    format!(
                        "{{ GRAPH <{src}> {{ {}?s <{parent}> <{item}> . ?s ?p ?o }} }}",
                        guard(&item)
                    ),
                ]
            })
            .collect();
        if branches.is_empty() {
            return Ok(Moved {
                ids: Vec::new(),
                subjects: 0,
                facts: 0,
                tx: 0,
            });
        }
        let update = format!(
            "DELETE {{ GRAPH <{src}> {{ ?s ?p ?o }} }} INSERT {{ GRAPH <{dst}> {{ ?s ?p ?o }} }} WHERE {{ {} }}",
            branches.join(" UNION ")
        );
        let source = if forward {
            "seeds:archive"
        } else {
            "seeds:unarchive"
        };
        let form = format!(
            "update={}&actor={}&source={}",
            form_encode(&update),
            form_encode(&ctx.actor),
            form_encode(source)
        );
        // A lost response may still have committed: never resend blind. The
        // verification below reads both graphs and decides what happened.
        #[cfg(test)]
        BEFORE_UPDATE.with(|hook| {
            if let Some(f) = hook.borrow().as_ref() {
                f(self);
            }
        });
        let (tx, lost) =
            match self.send("/update", "application/x-www-form-urlencoded", &form, true) {
                Ok(body) => (
                    update_report(&body).and_then(|(_, tx)| tx).unwrap_or(0),
                    None,
                ),
                Err((false, e)) => return Err(e),
                Err((true, e)) => (0, Some(e)),
            };
        let found_vec: Vec<String> = found.iter().cloned().collect();
        let after_src = self.family_facts(src, &found_vec)?;
        let after_dst = self.family_facts(dst, &found_vec)?;
        let found_iris: BTreeSet<String> = found.iter().map(|id| vocab::item_iri(id)).collect();
        let expected: Facts = before
            .iter()
            .filter(|(s, facts)| {
                found_iris.contains(*s)
                    || facts.iter().any(|(p, o)| {
                        *p == parent && matches!(o, Obj::Iri(i) if found_iris.contains(i))
                    })
            })
            .map(|(s, v)| (s.clone(), v.clone()))
            .collect();
        if let Some(e) = &lost {
            if after_src.is_empty() && after_dst == expected {
                eprintln!(
                    "sd: the server's response was lost ({}); a read-back shows the batch moved",
                    e.message
                );
            } else if after_dst.is_empty() && after_src == expected {
                return Err(SdError::failed(format!(
                    "the archive update did not land ({}); read-back shows nothing moved",
                    e.message
                )));
            }
        }
        if !after_src.is_empty() || after_dst != expected {
            return Err(SdError::failed(format!(
                "archive batch verification FAILED after tx {tx}: {} subject(s) left in the source, \
                 destination holds {} of {} expected subject(s). Stop: no further batch was sent. \
                 Batch ids: {}",
                after_src.len(),
                after_dst.len(),
                expected.len(),
                found_vec.join(" ")
            )));
        }
        Ok(Moved {
            ids: found_vec,
            subjects: expected.len(),
            facts: expected.values().map(Vec::len).sum(),
            tx,
        })
    }
}

/// Test-only fault injection: runs after the pre-read and before the
/// update is sent, so a test can change the graph in that gap.
#[cfg(test)]
type BeforeUpdate = Box<dyn Fn(&RemoteBackend)>;
#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_UPDATE: std::cell::RefCell<Option<BeforeUpdate>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
impl RemoteBackend {
    pub(super) fn family_facts_for_test(&self, graph: &str, items: &[String]) -> Facts {
        self.family_facts(graph, items).unwrap()
    }
}
