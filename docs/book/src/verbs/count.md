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
