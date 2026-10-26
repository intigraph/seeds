# sd count

Count seeds (closed ones only with `--include-closed` or `--status closed`).

```bash
sd count
sd count --by status --include-closed --json
```

| flag | meaning |
|---|---|
| `--by` | group by `status`, `priority`, `type`, `assignee` or `label` |
| `--status`, `--type`, `--assignee` | filters |
| `--include-closed` | count closed seeds too |

**`--json`**: `{"count": N}`; with `--by`, `{"total": N, "groups": [{"group",
"count"}]}`. With `--by label` a seed counts once per label, so the groups
can sum to more than the total.

`--by-status`, `--by-priority`, `--by-type`, `--by-assignee` and `--by-label`
are br's shorthands for `--by <x>` (one grouping at a time). `--priority`,
`--title-contains` and `--unassigned` filter what is counted.

For a current remote count, a compatible declared `board` projection can answer
assignee and priority filters together with a grouping. The client verifies
column meanings, normalized values and transaction consistency on every call.
An older server that lacks column filtering keeps the SPARQL reader; missing
priorities still mean P2, and `--unassigned` still selects missing assignees.
Other filters and data that require normalization also keep SPARQL. A timeout,
a refused request or a malformed response remains an error.

Projection compatibility is checked over a complete, bounded candidate set using
typed exact-subject fact reads. Candidates include missing-status items; only
positive filters that cannot hide normalized defaults narrow discovery. Unique
ID ownership is checked outside the selected filters. Incomplete reads fall back
to SPARQL; malformed responses and refused reads remain errors. All compatibility
reads stay inside the projection transaction bracket, including the check for
any ephemeral facts. The validated candidate count must match the aggregate.

The SPARQL fallback sends its total, groups and scalar-ambiguity check in one
`/query` request. A server that pins all reads of a query to one snapshot keeps
those results consistent across concurrent writes; a later count sees the new
snapshot. A single HTTP request alone cannot provide that guarantee on a server
without query snapshot support.

Competing scalar facts can make an aggregate select a different cohort from the
native model's first scalar value. The fallback refuses such ambiguity in the
fields relevant to the count instead of selecting any matching value. Labels
remain multivalued. Missing, truncated, malformed or inconsistent aggregate
results remain errors, never a zero count. Server query budgets still bound
evaluation; a client timeout or output limit is not proof of server cancellation.
