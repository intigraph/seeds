//! Indexed summaries: no ledger snapshot, seed body or comment materialization.
use super::*;
use crate::engine::{self, Activity, Count, CountReq, Filter, Stats, StatsReq};

fn literal(s: &str) -> String {
    format!("\"{}\"", escape_literal(s))
}

// Self-subtraction must be the integer term zero (not decimal/double zero),
// and adding integer zero must preserve term identity (not promote a typed
// integer subtype). Bound the range explicitly: newer Quipu versions preserve
// arbitrary-sized xsd:integer terms, but the snapshot decoder accepts i64 only.
// Checked i64 arithmetic retains MAX without f64 rounding.
pub(super) fn nonnegative_integer_guard(variable: &str) -> String {
    format!("sameTerm({variable} - {variable}, 0) && sameTerm({variable}, {variable} + 0) && {variable} >= 0 && {variable} <= 9223372036854775807")
}

fn principal_guard(variable: &str) -> String {
    format!("(isIRI({variable}) && STRSTARTS(STR({variable}), {})) || (isLiteral({variable}) && sameTerm({variable}, STR({variable})))", literal(&format!("{}principal/", vocab::SEEDS_BASE)))
}

fn title_field(field: &str) -> String {
    format!("OPTIONAL {{ ?s <{}> ?raw_{field}_name . FILTER(isLiteral(?raw_{field}_name) && sameTerm(?raw_{field}_name, STR(?raw_{field}_name))) }} OPTIONAL {{ ?s <{}> ?raw_{field}_label . FILTER(isLiteral(?raw_{field}_label) && sameTerm(?raw_{field}_label, STR(?raw_{field}_label))) }} BIND(COALESCE(?raw_{field}_name, ?raw_{field}_label, \"\") AS ?{field})", term::name(), vocab::RDFS_LABEL)
}

fn string_field(predicate: &str, field: &str, default: &str) -> String {
    format!("OPTIONAL {{ ?s <{predicate}> ?raw_{field} . FILTER(isLiteral(?raw_{field}) && sameTerm(?raw_{field}, STR(?raw_{field}))) }} BIND(COALESCE(?raw_{field}, {}) AS ?{field})", literal(default))
}

// Inline expressions retain only the two timestamp bindings: BINDing every
// calendar intermediate would multiply the aggregate's working set.
fn epoch_expression(variable: &str) -> String {
    let date = format!("STRDT(STR({variable}), <{}>)", vocab::XSD_DATE_TIME);
    let month = format!("MONTH({date})");
    let year = format!("(YEAR({date}) - IF({month} <= 2, 1, 0))");
    let era = format!("FLOOR({year} / 400.0)");
    let yoe = format!("({year} - {era} * 400)");
    let mp = format!("({month} + IF({month} <= 2, 9, -3))");
    let doy = format!("(FLOOR((153 * {mp} + 2) / 5.0) + DAY({date}) - 1)");
    let days = format!("((((({era} * 146097 + {yoe} * 365) + FLOOR({yoe} / 4.0)) - FLOOR({yoe} / 100.0)) + {doy}) - 719468)");
    format!(
        "({days} * 86400 + HOURS({date}) * 3600 + MINUTES({date}) * 60 + FLOOR(SECONDS({date})))"
    )
}

impl RemoteBackend {
    pub(super) fn search_summary(&self, req: &engine::SearchReq) -> Result<engine::SearchPage> {
        let needle = literal(&req.query.trim().to_lowercase());
        // Project only the metadata needed for filtering, counting and ordering.
        // A broad match must not turn into a point read for every matching seed.
        // Closed hits travel through the same decoder as visible hits, avoiding
        // a second, separately planned full-field aggregate.
        let (compact, has_ephemeral) = self.search_metadata_snapshot(req, &needle)?;
        let mut page = engine::search_matches_page(req, &compact)?;
        let ids = page
            .page
            .issues
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        let snap = self.page_snapshot(&ids, has_ephemeral)?;
        for seed in &mut page.page.issues {
            *seed = snap
                .seeds
                .get(&seed.id)
                .ok_or_else(|| {
                    SdError::failed("a search hit disappeared before its page was read")
                })?
                .clone();
        }
        page.page.dependent_counts = if req.full {
            self.incoming_counts_scoped(&ids, has_ephemeral)?
        } else {
            BTreeMap::new()
        };
        Ok(page)
    }

    fn search_branches(&self, needle: &str, full: bool, source_graph: &str) -> Vec<String> {
        // OPTIONAL followed by the binding guard has mandatory-join semantics.
        // Its projected right relation uses the keyed left-join evaluator even
        // when a broad match exceeds the subject bind-join work bound. A plain
        // Join would then fall back to comparing every pair of rows.
        let valid_id = format!(
            "OPTIONAL {{ {{ SELECT ?s ?id WHERE {{ ?s <{}> ?id }} }} }} FILTER(isLiteral(?id) && sameTerm(?id, STR(?id)))",
            term::identifier()
        );
        let eph = vocab::ephemeral_graph(&self.graph);
        let shadow = if source_graph == self.graph {
            format!(
                "FILTER NOT EXISTS {{ GRAPH <{eph}> {{ ?shadow a <{}> ; <{}> ?id }} }}",
                term::work_item(),
                term::identifier()
            )
        } else {
            String::new()
        };
        let predicates = if full {
            vec![
                term::identifier(),
                term::name(),
                vocab::RDFS_LABEL.into(),
                term::description(),
            ]
        } else {
            vec![term::name(), vocab::RDFS_LABEL.into(), term::description()]
        };
        let mut branches = predicates.into_iter().map(|p| {
            let fallback = if p == vocab::RDFS_LABEL { format!("FILTER NOT EXISTS {{ ?s <{}> ?name FILTER(isLiteral(?name) && sameTerm(?name, STR(?name))) }}",term::name()) } else { String::new() };
            {
                // Project matching subjects before joining their metadata. Keeping
                // the text binding out of those joins avoids retaining every
                // description through type/status/shadow evaluation.
                format!("{{ {{ SELECT ?s WHERE {{ ?s <{p}> ?text . FILTER(isLiteral(?text) && sameTerm(?text, STR(?text)) && CONTAINS(LCASE(STR(?text)), {needle})) }} }} {valid_id} FILTER EXISTS {{ ?s a <{}> . {fallback} {shadow} }} }}", term::work_item())
            }
        }).collect::<Vec<_>>();
        if full {
            // Parent metadata is decoded after identities from both graphs
            // have been combined. Joining parent identifiers, status and
            // shadowing into every comment match repeats that work and loses
            // inherited comments when the parent's effective graph differs.
            // Project each validation relation once and use the same keyed
            // OPTIONAL + binding-guard form as the identifier lookup above.
            branches.push(format!("{{ {{ SELECT DISTINCT ?s WHERE {{ {{ SELECT ?comment WHERE {{ ?comment <{}> ?text . FILTER(isLiteral(?text) && sameTerm(?text, STR(?text)) && CONTAINS(LCASE(STR(?text)), {needle})) }} }} OPTIONAL {{ {{ SELECT ?comment ?s WHERE {{ ?comment <{}> ?s }} }} }} OPTIONAL {{ {{ SELECT ?comment ?valid_comment WHERE {{ ?comment a <{}> . BIND(\"valid\" AS ?valid_comment) }} }} }} OPTIONAL {{ {{ SELECT ?comment ?index WHERE {{ ?comment <{}> ?index }} }} }} FILTER(isIRI(?s) && STRSTARTS(STR(?s), {}) && ?valid_comment = \"valid\" && ({})) }} }} }}",term::text(),term::comment_on(),term::comment(),term::comment_index(),literal(&format!("{}item/",vocab::SEEDS_BASE)),nonnegative_integer_guard("?index")));
        }

        branches
    }

    fn metadata_predicates(
        filter: &Filter,
        sort: Option<&str>,
        defer_until_present: bool,
    ) -> Vec<String> {
        let mut predicates = vec![vocab::RDF_TYPE.into(), term::identifier(), term::status()];
        if defer_until_present {
            predicates.push(term::defer_until());
        }
        let sort = sort.unwrap_or("priority");
        if matches!(sort, "priority" | "priority,rank")
            || filter.priority.is_some()
            || filter.priority_min.is_some()
            || filter.priority_max.is_some()
        {
            predicates.push(term::priority());
        }
        if matches!(sort, "priority" | "priority,rank" | "created") {
            predicates.push(term::created_at());
        }
        if sort == "priority,rank" {
            predicates.extend([term::rank(), term::due_at()]);
        }
        if sort == "updated" {
            predicates.push(term::updated_at());
        }
        if sort == "title" || filter.title_contains.is_some() {
            predicates.extend([term::name(), vocab::RDFS_LABEL.into()]);
        }
        if filter.assignee.is_some() || filter.unassigned {
            predicates.push(term::assigned_to());
        }
        if !filter.labels.is_empty() || !filter.labels_any.is_empty() {
            predicates.push(term::label());
        }
        if filter.issue_type.is_some() {
            predicates.push(term::issue_type());
        }
        if filter.parent.is_some() {
            predicates.push(term::child_of());
        }
        for (wanted, p) in [
            (filter.desc_contains.is_some(), term::description()),
            (filter.notes_contains.is_some(), term::notes()),
            (filter.overdue_at.is_some(), term::due_at()),
        ] {
            if wanted {
                predicates.push(p);
            }
        }
        predicates
    }

    /// Read matching metadata as wide, ordered rows, not one triple per field
    /// and one UNION branch per subject. Bodies and comments never enter this
    /// projection unless an explicit metadata filter needs a body field.
    fn search_metadata_snapshot(
        &self,
        req: &engine::SearchReq,
        needle: &str,
    ) -> Result<(Snapshot, bool)> {
        let predicates = Self::metadata_predicates(&req.filter, req.sort.as_deref(), false);
        let mut main = Facts::new();
        let mut eph = Facts::new();
        let has_ephemeral = self.ephemeral_may_hold_facts()?;
        // Matching comments can live in either graph while their effective
        // parent lives in the other. Select identities across both graphs
        // before reading either version of their metadata.
        let graphs = self.both_graphs();
        let matching = graphs
            .iter()
            .map(|graph| {
                let branches = self
                    .search_branches(needle, req.full, graph)
                    .join(" UNION ");
                format!("{{ GRAPH <{graph}> {{ {branches} }} }}")
            })
            .collect::<Vec<_>>()
            .join(" UNION ");
        let mut columns = Vec::new();
        let mut fields = Vec::new();
        for (origin, graph) in graphs
            .iter()
            .enumerate()
            .take(if has_ephemeral { 2 } else { 1 })
        {
            for (i, predicate) in predicates.iter().enumerate() {
                let variable = format!("?m{origin}_{i}");
                columns.push(variable.clone());
                fields.push(format!(
                    "OPTIONAL {{ GRAPH <{graph}> {{ ?s <{predicate}> {variable} }} }}"
                ));
            }
        }
        let columns = columns.join(" ");
        let fields = fields.join(" ");
        let page_size = if self.subject_page < SUBJECT_PAGE {
            self.subject_page
        } else {
            ROW_CAP - 1
        };
        let mut lower: Option<String> = None;
        loop {
            let bound = lower
                .as_ref()
                .map(|s| format!("FILTER(STR(?s) > {})", literal(s)))
                .unwrap_or_default();
            // Limit identities before OPTIONAL metadata. Keeping each lookup
            // beside the bounded identity table lets the evaluator bind its
            // subject; a separate UNION of whole-graph metadata would first
            // materialize both graphs and only then discard unmatched rows.
            let query = format!(
                "SELECT ?s {columns} WHERE {{ {{ SELECT DISTINCT ?s WHERE {{ {matching} FILTER(isIRI(?s)) {bound} }} ORDER BY STR(?s) LIMIT {page_size} }} {fields} }} ORDER BY STR(?s) LIMIT {page_size}");
            let (rows, truncated) = self.select_marked(&query, None)?;
            if truncated || rows.len() > page_size {
                return Err(SdError::failed("quipu truncated matching search metadata"));
            }
            let mut last: Option<&String> = None;
            for row in &rows {
                let Some(Obj::Iri(subject)) = row.get("s") else {
                    return Err(SdError::failed(
                        "quipu returned an invalid search metadata subject",
                    ));
                };
                if lower.as_ref().is_some_and(|s| subject <= s) || last.is_some_and(|s| subject < s)
                {
                    return Err(SdError::failed("quipu returned unordered search metadata"));
                }
                last = Some(subject);
            }
            let complete = rows.len() < page_size;
            // A full page may split a subject's multi-valued fields or its
            // project/ephemeral versions. Read that boundary identity's raw
            // metadata before advancing, preserving both complete versions.
            let boundary = if complete { None } else { last.cloned() };
            for row in &rows {
                let Some(Obj::Iri(subject)) = row.get("s") else {
                    unreachable!()
                };
                if boundary.as_ref() == Some(subject) {
                    continue;
                }
                for (origin, facts) in [&mut main, &mut eph]
                    .into_iter()
                    .enumerate()
                    .take(if has_ephemeral { 2 } else { 1 })
                {
                    let target = facts.entry(subject.clone()).or_default();
                    for (i, predicate) in predicates.iter().enumerate() {
                        if let Some(value) = row.get(&format!("m{origin}_{i}")) {
                            let fact = (predicate.clone(), value.clone());
                            if !target.contains(&fact) {
                                target.push(fact);
                            }
                        }
                    }
                }
            }
            if complete {
                break;
            }
            let boundary =
                boundary.ok_or_else(|| SdError::failed("search metadata made no progress"))?;
            self.read_metadata(&self.graph, &[&boundary], &predicates, &mut main)?;
            if has_ephemeral {
                self.read_metadata(&graphs[1], &[&boundary], &predicates, &mut eph)?;
            }
            lower = Some(boundary);
        }
        Ok((Snapshot::from_graphs(&main, &eph), has_ephemeral))
    }

    fn metadata_snapshot(
        &self,
        subjects: &BTreeSet<String>,
        filter: &Filter,
        sort: Option<&str>,
        defer_until_present: bool,
    ) -> Result<(Snapshot, bool)> {
        let mut main = Facts::new();
        let mut eph = Facts::new();
        let mut has_ephemeral = false;
        let predicates = Self::metadata_predicates(filter, sort, defer_until_present);
        for (graph, facts) in [
            (self.graph.clone(), &mut main),
            (vocab::ephemeral_graph(&self.graph), &mut eph),
        ] {
            if graph != self.graph {
                has_ephemeral = self.ephemeral_may_hold_facts()?;
                if !has_ephemeral {
                    continue;
                }
            }
            let ids = subjects.iter().collect::<Vec<_>>();
            // Bound both payload and parser structural cost (default server cap 4096).
            let per_read = (3600 / (predicates.len() * 4 + 3)).max(1);
            for chunk in ids.chunks(per_read) {
                self.read_metadata(&graph, chunk, &predicates, facts)?;
            }
        }
        Ok((Snapshot::from_graphs(&main, &eph), has_ephemeral))
    }

    fn page_snapshot(&self, ids: &[String], has_ephemeral: bool) -> Result<Snapshot> {
        let subjects = ids.iter().map(|id| vocab::item_iri(id)).collect::<Vec<_>>();
        self.full_snapshot(&subjects.iter().collect::<Vec<_>>(), has_ephemeral)
    }

    /// The full current facts of `subjects` in the project graph (and the
    /// ephemeral graph, when it may hold facts), as one snapshot.
    fn full_snapshot(&self, subjects: &[&String], has_ephemeral: bool) -> Result<Snapshot> {
        // Seeds whose full facts one read asks for. A seed averages ~19 current
        // facts on the live board, so 200 stay well under quipu's 10,000-row
        // cap; read_subjects still halves any read that is truncated or times
        // out. At 13 per read, `sd list --all` made ~13,500 round trips for
        // 87,782 seeds (aegis-ek4.1).
        const PAGE_SUBJECTS_PER_READ: usize = 200;
        let mut main = Facts::new();
        let mut eph = Facts::new();
        // Reuse the ephemeral-graph observation from this listing, never
        // across commands or backend calls.
        for chunk in subjects.chunks(PAGE_SUBJECTS_PER_READ) {
            self.read_subjects(&self.graph, chunk, None, &mut main)?;
            if has_ephemeral {
                self.read_subjects(&vocab::ephemeral_graph(&self.graph), chunk, None, &mut eph)?;
            }
        }
        Ok(Snapshot::from_graphs(&main, &eph))
    }

    pub(super) fn list_summary(&self, req: &engine::ListReq) -> Result<engine::Page> {
        let q = engine::listing_seed_query(req)?;
        // Start from the selective field index alone. Status/default UNION
        // joins can materialize the whole Action class before this restriction.
        // Metadata decoding and list_matches_page validate type and all filters.
        let text_filter = req
            .filter
            .desc_contains
            .as_ref()
            .map(|text| (term::description(), text))
            .or_else(|| {
                req.filter
                    .notes_contains
                    .as_ref()
                    .map(|text| (term::notes(), text))
            });
        let pattern = if req.defer_until_present {
            format!("?s <{}> ?defer_candidate .", term::defer_until())
        } else if let Some((predicate, text)) = text_filter {
            // A field may match while another filter rejects its seed. This
            // is only a candidate superset: the existing decoder and metadata
            // filter still decide membership, defaults, ordering and totals.
            format!("?s <{predicate}> ?text_candidate . FILTER(isLiteral(?text_candidate) && sameTerm(?text_candidate, STR(?text_candidate)) && CONTAINS(LCASE(STR(?text_candidate)), {}))", literal(&text.to_lowercase()))
        } else {
            String::new()
        };
        let mut subjects = BTreeSet::new();
        if req.filter.ids.is_empty() {
            for graph in self.both_graphs() {
                let scoped = if pattern.is_empty() {
                    self.owner_seed_query_pattern(&graph, &q)?
                        .unwrap_or_else(|| format!("?s a <{}>", term::work_item()))
                } else {
                    pattern.clone()
                };
                subjects.extend(self.subjects_where(&graph, &scoped, None)?);
            }
        } else {
            subjects.extend(req.filter.ids.iter().map(|id| vocab::item_iri(id)));
        }
        // A listing that takes everything (limit 0, no offset) reads every
        // candidate's full facts for its page anyway. Filter and sort on those
        // full facts directly instead of reading the metadata first: the full
        // facts hold every predicate the metadata read selects, so the same
        // seeds match in the same order (aegis-ek4.1: ~10s of an export).
        if req.limit == Some(0) && req.offset == 0 {
            let has_ephemeral = self.ephemeral_may_hold_facts()?;
            let full = self.full_snapshot(&subjects.iter().collect::<Vec<_>>(), has_ephemeral)?;
            let mut page = engine::list_matches_page(req, &full)?;
            page.dependent_counts = self.listed_dependent_counts(req, &page, has_ephemeral)?;
            return Ok(page);
        }
        let (compact, has_ephemeral) = self.metadata_snapshot(
            &subjects,
            &req.filter,
            req.sort.as_deref(),
            req.defer_until_present,
        )?;
        let mut page = engine::list_matches_page(req, &compact)?;
        let ids = page.issues.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        let snap = self.page_snapshot(&ids, has_ephemeral)?;
        for seed in &mut page.issues {
            *seed = snap
                .seeds
                .get(&seed.id)
                .ok_or_else(|| {
                    SdError::failed("a listed seed disappeared before its page was read")
                })?
                .clone();
        }
        page.dependent_counts = self.listed_dependent_counts(req, &page, has_ephemeral)?;
        Ok(page)
    }

    /// The page's dependent counts, unless the caller will not show them.
    fn listed_dependent_counts(
        &self,
        req: &engine::ListReq,
        page: &engine::Page,
        has_ephemeral: bool,
    ) -> Result<BTreeMap<String, usize>> {
        if req.skip_dependent_counts {
            return Ok(BTreeMap::new());
        }
        let ids = page.issues.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        self.incoming_counts_scoped(&ids, has_ephemeral)
    }

    pub(super) fn incoming_counts(&self, ids: &[String]) -> Result<BTreeMap<String, usize>> {
        self.incoming_counts_scoped(ids, true)
    }

    /// Bound branches in one dependent-count read.
    const COUNT_BRANCHES_PER_READ: usize = 400;

    fn incoming_counts_scoped(
        &self,
        ids: &[String],
        has_ephemeral: bool,
    ) -> Result<BTreeMap<String, usize>> {
        let mut counts = ids
            .iter()
            .map(|id| (id.clone(), 0))
            .collect::<BTreeMap<_, _>>();
        let edges = [
            term::blocked_on(),
            term::child_of(),
            term::related_to(),
            term::discovered_from(),
        ];
        let eph = vocab::ephemeral_graph(&self.graph);
        // About 400 bound branches per read (four edges, one or two graphs),
        // far under quipu's structural cap; each branch is an indexed lookup
        // on a bound target, and grouped() still refuses a truncated answer.
        // At 8 targets per read `sd list --all` made ~11,000 count reads for
        // 87,782 seeds (aegis-ek4.1).
        let graphs = if has_ephemeral { 2 } else { 1 };
        let per_read = (Self::COUNT_BRANCHES_PER_READ / (edges.len() * graphs)).max(1);
        for chunk in ids.chunks(per_read) {
            let mut branches = Vec::new();
            for id in chunk {
                for graph in self
                    .both_graphs()
                    .into_iter()
                    .take(if has_ephemeral { 2 } else { 1 })
                {
                    for edge in &edges {
                        let prefer = if has_ephemeral && graph == self.graph {
                            format!("FILTER NOT EXISTS {{ GRAPH <{eph}> {{ ?shadow a <{}> ; <{}> ?source_id }} }}",term::work_item(),term::identifier())
                        } else {
                            String::new()
                        };
                        branches.push(format!("{{ GRAPH <{graph}> {{ ?s <{edge}> <{}> ; a <{}> ; <{}> ?source_id }} {prefer} BIND({} AS ?target) BIND(CONCAT(STR(?source_id), {}) AS ?id) }}",vocab::item_iri(id),term::work_item(),term::identifier(),literal(id),literal(&format!("/{edge}"))));
                    }
                }
            }
            for (id, n) in self.grouped(&branches.join(" UNION "), "target")? {
                counts.insert(id, n);
            }
        }
        Ok(counts)
    }

    fn read_metadata(
        &self,
        graph: &str,
        subjects: &[&String],
        predicates: &[String],
        facts: &mut Facts,
    ) -> Result<()> {
        if subjects.is_empty() {
            return Ok(());
        }
        let branches = subjects
            .iter()
            .map(|s| {
                let properties = predicates
                    .iter()
                    .map(|p| {
                        format!("{{ BIND(<{p}> AS ?p) GRAPH <{graph}> {{ <{s}> <{p}> ?o }} }}")
                    })
                    .collect::<Vec<_>>();
                format!(
                    "{{ BIND(<{s}> AS ?s) {{ {} }} }}",
                    properties.join(" UNION ")
                )
            })
            .collect::<Vec<_>>();
        let (rows, truncated) = self.select_marked(
            &format!(
                "SELECT ?s ?p ?o WHERE {{ {} }} LIMIT {ROW_CAP}",
                branches.join(" UNION ")
            ),
            None,
        )?;
        if truncated || rows.len() >= ROW_CAP {
            if subjects.len() == 1 {
                return Err(SdError::failed("quipu truncated one seed's metadata"));
            }
            let (a, b) = subjects.split_at(subjects.len() / 2);
            self.read_metadata(graph, a, predicates, facts)?;
            return self.read_metadata(graph, b, predicates, facts);
        }
        for mut row in rows {
            if let (Some(Obj::Iri(s)), Some(Obj::Iri(p)), Some(o)) =
                (row.remove("s"), row.remove("p"), row.remove("o"))
            {
                facts.entry(s).or_default().push((p, o));
            }
        }
        Ok(())
    }

    /// Ephemeral seeds replace project seeds of the same id, as Snapshot does.
    ///
    /// Required `drive` triples go BEFORE the work-item type triple: Quipu
    /// evaluates a pattern holding a type triple in written order, so a
    /// selective constant written first is what lets it start from the cohort
    /// instead of every seed (aegis-efa).
    fn effective_seeds_driven(&self, drive: &str, fields: &str) -> String {
        let eph = vocab::ephemeral_graph(&self.graph);
        let base = format!(
            "{drive} ?s a <{}> ; <{}> ?id .",
            term::work_item(),
            term::identifier()
        );
        format!("{{ GRAPH <{}> {{ {base} {fields} FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) }} FILTER NOT EXISTS {{ GRAPH <{eph}> {{ ?shadow a <{}> ; <{}> ?id }} }} }} UNION {{ GRAPH <{eph}> {{ {base} {fields} FILTER(isLiteral(?id) && sameTerm(?id, STR(?id))) }} }}", self.graph, term::work_item(), term::identifier())
    }

    fn status_fields(&self) -> String {
        format!(
            "?s <{}> ?status . FILTER(isLiteral(?status) && sameTerm(?status, STR(?status)))",
            term::status()
        )
    }

    fn effective_normalized(&self, fields: &str) -> String {
        self.effective_normalized_driven("", fields)
    }

    fn effective_normalized_driven(&self, drive: &str, fields: &str) -> String {
        let regular = self.effective_seeds_driven(drive, fields);
        let missing = format!("FILTER NOT EXISTS {{ ?s <{}> ?raw_status FILTER(isLiteral(?raw_status) && sameTerm(?raw_status, STR(?raw_status))) }} BIND(\"open\" AS ?status)",term::status());
        let fallback =
            self.effective_seeds_driven(drive, &fields.replace(&self.status_fields(), &missing));
        format!("{{ {regular} }} UNION {{ {fallback} }}")
    }

    fn grouped(&self, pattern: &str, key: &str) -> Result<Vec<(String, usize)>> {
        let (rows, truncated) = self.select_marked(&format!("SELECT ?{key} (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {pattern} }} GROUP BY ?{key} LIMIT {ROW_CAP}"), None)?;
        if truncated || rows.len() >= ROW_CAP {
            return Err(SdError::failed(
                "quipu truncated grouped counts; refusing an incomplete summary",
            ));
        }
        let mut groups = BTreeMap::new();
        for row in rows {
            let value = match row.get(key) {
                Some(Obj::Str(s)) => s.clone(),
                Some(Obj::Iri(s)) => vocab::principal_name(s).unwrap_or_else(|| s.clone()),
                Some(Obj::Int(n)) => n.to_string(),
                _ => return Err(SdError::failed("quipu returned an invalid group key")),
            };
            let n = match row.get("n") {
                Some(Obj::Int(n)) => usize::try_from(*n).ok(),
                Some(Obj::Str(n)) => n.parse().ok(),
                _ => None,
            }
            .ok_or_else(|| SdError::failed("quipu returned an invalid grouped count"))?;
            *groups.entry(value).or_default() += n;
        }
        Ok(groups.into_iter().collect())
    }

    /// Required triples that select an explicit assignee and/or a single
    /// non-default priority. Each is equivalent to the OPTIONAL + FILTER form
    /// it replaces: that FILTER rejects every row whose value is absent, and
    /// the default priority (2, applied to seeds without one) is excluded.
    fn filter_drive(f: &Filter) -> Result<(String, bool, bool)> {
        let mut drive = String::new();
        let assignee = f.assignee.is_some() && !f.unassigned;
        if let (true, Some(person)) = (assignee, &f.assignee) {
            drive += &format!(
                "VALUES ?assignee {{ <{}> {} }} ?s <{}> ?assignee .",
                vocab::principal_iri(person),
                literal(person),
                term::assigned_to()
            );
        }
        let mut priority = false;
        if let (Some(value), None, None) = (&f.priority, &f.priority_min, &f.priority_max) {
            let value = crate::model::parse_priority(value)?;
            if value != 2 {
                drive += &format!(" ?s <{}> {value} .", term::priority());
                priority = true;
            }
        }
        Ok((drive.trim().to_string(), assignee, priority))
    }

    fn filter_parts(&self, f: &Filter) -> Result<(String, String)> {
        let (drive, driven_assignee, driven_priority) = Self::filter_drive(f)?;
        let mut fields = self.status_fields();
        if let Some(status) = &f.status {
            fields += &format!(
                " FILTER(?status = {})",
                literal(&crate::model::parse_status(status)?)
            );
        }
        if let Some(kind) = &f.issue_type {
            fields += &string_field(&term::issue_type(), "kind", "task");
            fields += &format!(
                " FILTER(?kind = {})",
                literal(&crate::model::parse_type(kind)?)
            );
        }
        if (f.assignee.is_some() || f.unassigned) && !driven_assignee {
            fields += &format!(
                " OPTIONAL {{ ?s <{}> ?assignee . FILTER({}) }}",
                term::assigned_to(),
                principal_guard("?assignee")
            );
            if let Some(person) = &f.assignee {
                fields += &format!(
                    " FILTER(?assignee = <{}> || ?assignee = {})",
                    vocab::principal_iri(person),
                    literal(person)
                );
            }
            if f.unassigned {
                fields += " FILTER(!BOUND(?assignee))";
            }
        }
        for label in f.labels.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            fields += &format!(" ?s <{}> {} .", term::label(), literal(label.trim()));
        }
        let labels_any = f
            .labels_any
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !labels_any.is_empty() {
            fields += &format!(
                " ?s <{}> ?label_any . FILTER(?label_any IN ({}))",
                term::label(),
                labels_any
                    .iter()
                    .map(|s| literal(s.trim()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if (f.priority.is_some() || f.priority_min.is_some() || f.priority_max.is_some())
            && !driven_priority
        {
            fields += &format!(" OPTIONAL {{ ?s <{}> ?raw_priority . FILTER({} && ?raw_priority <= 255) }} BIND(COALESCE(?raw_priority, 2) AS ?priority)", term::priority(), nonnegative_integer_guard("?raw_priority"));
            for (value, operator) in [
                (&f.priority, "="),
                (&f.priority_min, ">="),
                (&f.priority_max, "<="),
            ] {
                if let Some(value) = value {
                    fields += &format!(
                        " FILTER(?priority {operator} {})",
                        crate::model::parse_priority(value)?
                    );
                }
            }
        }
        if let Some(parent) = &f.parent {
            fields += &format!(" ?s <{}> <{}> .", term::child_of(), vocab::item_iri(parent));
        }
        if !f.ids.is_empty() {
            fields += &format!(
                " FILTER(?id IN ({}))",
                f.ids
                    .iter()
                    .map(|s| literal(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        for (text, predicate, key) in [
            (&f.title_contains, term::name(), "title"),
            (&f.desc_contains, term::description(), "description"),
            (&f.notes_contains, term::notes(), "notes"),
        ] {
            if let Some(text) = text {
                if key == "title" {
                    fields += &title_field(key);
                } else {
                    // Optional text is absent unless the decoder sees a plain
                    // string, including when an empty substring was requested.
                    fields += &format!(" ?s <{predicate}> ?{key} . FILTER(isLiteral(?{key}) && sameTerm(?{key}, STR(?{key})))");
                }
                fields += &format!(
                    " FILTER(CONTAINS(LCASE(STR(?{key})), {}))",
                    literal(&text.to_lowercase())
                );
            }
        }
        if let Some(now) = &f.overdue_at {
            fields += &format!(" ?s <{}> ?due . FILTER(STR(?due) < {} && ?status != \"closed\" && ?status != \"tombstone\")", term::due_at(), literal(now));
        }
        Ok((drive, fields))
    }

    fn group_field(by: &str) -> Result<String> {
        Ok(match by {
            "status" => String::new(),
            "type" => string_field(&term::issue_type(), "key", "task"),
            "priority" => format!("OPTIONAL {{ ?s <{}> ?raw_key . FILTER({} && ?raw_key <= 255) }} BIND(CONCAT(\"P\", STR(COALESCE(?raw_key, 2))) AS ?key)", term::priority(), nonnegative_integer_guard("?raw_key")),
            "assignee" => format!("OPTIONAL {{ ?s <{}> ?raw_key . FILTER({}) }} BIND(COALESCE(?raw_key, \"(unassigned)\") AS ?key)", term::assigned_to(), principal_guard("?raw_key")),
            "label" => string_field(&term::label(), "key", "(no labels)"),
            _ => return Err(SdError::usage("unknown --by; expected status, priority, type, assignee or label")),
        })
    }

    pub(super) fn count_summary(&self, req: &CountReq) -> Result<Count> {
        self.count_summary_with_source(req).map(|(count, _)| count)
    }

    pub(super) fn count_summary_with_source(
        &self,
        req: &CountReq,
    ) -> Result<(Count, projection::CountSource)> {
        if let Some(count) = self.projection_count(req)? {
            return Ok((count, projection::CountSource::Projection));
        }
        self.sparql_count_summary(req)
            .map(|count| (count, projection::CountSource::Sparql))
    }

    pub(super) fn sparql_count_summary(&self, req: &CountReq) -> Result<Count> {
        let (drive, mut fields) = self.filter_parts(&req.filter)?;
        if req.filter.status.is_none() {
            fields += " FILTER(?status != \"tombstone\")";
            if !req.include_closed {
                fields += " FILTER(?status != \"closed\")";
            }
        }
        let total = self.aggregate_number(&format!(
            "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {} }}",
            self.effective_normalized_driven(&drive, &fields)
        ))?;
        let groups = req
            .by
            .as_deref()
            .map(|by| {
                let key = if by == "status" { "status" } else { "key" };
                self.grouped(
                    &self.effective_normalized_driven(
                        &drive,
                        &(fields.clone() + &Self::group_field(by)?),
                    ),
                    key,
                )
            })
            .transpose()?;
        Ok(Count { total, groups })
    }

    pub(super) fn stats_summary(&self, ctx: &Ctx, req: StatsReq) -> Result<Stats> {
        let statuses = match self.projection_status_totals()? {
            Some(statuses) => statuses,
            None => self.grouped(&self.effective_normalized(&self.status_fields()), "status")?,
        };
        let count = |status: &str| {
            statuses
                .iter()
                .find(|(key, _)| key == status)
                .map_or(0, |(_, n)| *n)
        };
        let mut stats = Stats {
            total: statuses
                .iter()
                .filter(|(key, _)| key != crate::model::TOMBSTONE)
                .map(|(_, n)| n)
                .sum(),
            open: count("open"),
            in_progress: count("in_progress"),
            closed: count("closed"),
            blocked: count("blocked"),
            deferred: count("deferred"),
            tombstones: count(crate::model::TOMBSTONE),
            ..Stats::default()
        };
        let active = self.status_fields() + " FILTER(?status != \"tombstone\")";
        for (wanted, by) in [
            (req.by_type, "type"),
            (req.by_priority, "priority"),
            (req.by_assignee, "assignee"),
            (req.by_label, "label"),
        ] {
            if wanted {
                stats.breakdowns.push((
                    by,
                    self.grouped(
                        &self.effective_normalized(&(active.clone() + &Self::group_field(by)?)),
                        "key",
                    )?,
                ));
            }
        }
        // Compute the mean on the server, without transferring closed-seed
        // facts. Match the core's whole-second Gregorian calendar arithmetic.
        let duration = format!(
            "(({}) - ({})) / 3600.0",
            epoch_expression("?closed"),
            epoch_expression("?created")
        );
        let pattern = format!(
            "?s <{}> \"closed\" ; a <{}> ; <{}> ?id ; <{}> ?created ; <{}> ?closed .",
            term::status(),
            term::work_item(),
            term::identifier(),
            term::created_at(),
            term::closed_at()
        );
        let eph = vocab::ephemeral_graph(&self.graph);
        let graphs = format!("{{ GRAPH <{}> {{ {pattern} }} FILTER NOT EXISTS {{ GRAPH <{eph}> {{ ?shadow a <{}> ; <{}> ?id }} }} }} UNION {{ GRAPH <{eph}> {{ {pattern} }} }}",self.graph,term::work_item(),term::identifier());
        let (rows, truncated) = self.select_marked(
            &format!(
                "SELECT (AVG({duration}) AS ?mean) (COUNT({duration}) AS ?n) WHERE {{ {graphs} }}"
            ),
            None,
        )?;
        if truncated || rows.len() != 1 {
            return Err(SdError::failed(
                "quipu returned an incomplete lead-time aggregate",
            ));
        }
        let n = rows[0]
            .get("n")
            .and_then(|v| match v {
                Obj::Int(n) => usize::try_from(*n).ok(),
                Obj::Str(n) => n.parse().ok(),
                _ => None,
            })
            .ok_or_else(|| SdError::failed("quipu returned an invalid lead-time count"))?;
        stats.average_lead_time_hours = if n == 0 {
            0.0
        } else {
            rows[0]
                .get("mean")
                .and_then(|v| match v {
                    Obj::Int(n) => Some(*n as f64),
                    Obj::Str(s) | Obj::Typed { lexical: s, .. } => s.parse::<f64>().ok(),
                    _ => None,
                })
                .filter(|n| n.is_finite())
                .ok_or_else(|| SdError::failed("quipu returned an invalid lead-time mean"))?
        };
        let mut ready = vocab::ready_query(&self.graph)
            .replace("SELECT ?id", "SELECT (COUNT(DISTINCT ?id) AS ?n)");
        let tail = "OPTIONAL { ?item <".to_string()
            + &term::defer_until()
            + "> ?defer } FILTER(!BOUND(?defer) || STR(?defer) <= "
            + &literal(&ctx.now)
            + ")";
        let position = ready.rfind("}} ").or_else(|| ready.rfind("}}"));
        // The public query ends with the GRAPH and WHERE braces on separate lines.
        let position = position.or_else(|| ready.rfind("} }"));
        if let Some(position) = position {
            ready.insert_str(position, &tail);
        } else {
            return Err(SdError::failed("cannot construct the ready aggregate"));
        }
        stats.ready = self.aggregate_number(&ready)?;
        let fields = format!("?s <{}> \"epic\" . {active} FILTER(?status != \"closed\") FILTER EXISTS {{ ?child <{}> ?s ; <{}> \"closed\" . ?s a <{}> }} FILTER NOT EXISTS {{ ?child <{}> ?s . FILTER NOT EXISTS {{ ?child <{}> \"closed\" }} FILTER NOT EXISTS {{ ?child <{}> \"tombstone\" }} }}", term::issue_type(), term::child_of(), term::status(), term::work_item(),term::child_of(), term::status(),term::status());
        stats.epics_eligible_for_closure = self.aggregate_number(&format!(
            "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {} }}",
            self.effective_normalized(&fields)
        ))?;
        if let Some(hours) = req.activity_hours {
            let since = engine::parse_since(&format!("{hours}h"), &ctx.now)?;
            let time_count = |predicate: String, extra: &str| {
                let fields = format!(
                    "?s <{predicate}> ?time . {active} FILTER(STR(?time) >= {}) {extra}",
                    literal(&since)
                );
                self.aggregate_number(&format!(
                    "SELECT (COUNT(DISTINCT ?id) AS ?n) WHERE {{ {} }}",
                    self.effective_normalized(&fields)
                ))
            };
            let updated_extra = format!("FILTER NOT EXISTS {{ ?s <{}> ?created FILTER(STR(?created) >= {}) }} FILTER NOT EXISTS {{ ?s <{}> ?closed FILTER(STR(?closed) >= {}) }}", term::created_at(), literal(&since), term::closed_at(), literal(&since));
            stats.activity = Some(Activity {
                hours,
                created: time_count(term::created_at(), "")?,
                closed: time_count(term::closed_at(), "")?,
                updated: time_count(term::updated_at(), &updated_extra)?,
                touched: time_count(term::updated_at(), "")?,
            });
        }
        Ok(stats)
    }
}
