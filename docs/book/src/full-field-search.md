# Remote full-field search

`sd search TERM` retains its title-only default. Explicit `--full` additionally
matches identifiers, descriptions and valid comments, with the same filters,
closed-item counts, sorting and pagination as the snapshot implementation.

For current remote reads, the server must support the explicit substring-index
contract and have a complete compatible index. A cheap readiness request precedes
text selection. Each subsequent request requires the same contract; a missing
acknowledgement or unavailable index refuses rather than materializing a board
snapshot. No positive or negative readiness result is cached between searches.
Terms shorter than three Unicode characters after lowercasing, or containing NUL,
refuse before query evaluation because this index cannot accelerate them. Default
title-only searches retain support for short terms.

The backend projects matching subjects before joining compact metadata, validates
comment parents and integer positions, and deduplicates matching comment parents
before item eligibility checks. Main and ephemeral graphs retain their effective
shadowing rules. Only the selected page's item bodies are hydrated.

An operator must explicitly prepare the optional index using the matching Quipu
maintenance tooling. A source update does not authorize production backfill.
Dropping it makes requiring searches refuse; ordinary title searches retain their
existing contract. Historical reads retain the existing snapshot read-cost gate.

The public synthetic demo and protocol contract live in Quipu's
`docs/probes/pr-demos/substring-search/` and
`docs/book/src/reference/substring-search.md`.
