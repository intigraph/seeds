# Moving from beads

seeds answers the verbs agents already type for beads (`bd`) and beads_rust
(`br`), with `--json` in bd's output shape. This page is the migration path
from a current, Dolt-backed bd board, and the audit behind the command table
in the README.

## Import a bd board

Measured against upstream bd 1.0.5 (Dolt backend, embedded engine). A full
read of the issues table is `bd export`: one JSON object per issue, with its
labels, dependencies and comments embedded. seeds reads that JSONL through
the lossless beads bridge ([Cutover](verbs/cutover.md)); it does not open the
Dolt database itself.

```bash
# In the bd workspace
bd export -o issues.jsonl

# In the seeds project: rehearse, then import
sd --store .seeds/seeds.db --graph https://seeds.local/project/myproject \
   cutover verify --file issues.jsonl            # isolated in-memory store; reports losses
sd --store .seeds/seeds.db --graph https://seeds.local/project/myproject \
   cutover import --file issues.jsonl --dry-run  # would-change ids and fields, writes nothing
sd --store .seeds/seeds.db --graph https://seeds.local/project/myproject \
   cutover import --file issues.jsonl            # one quipu transaction
```

What comes across, with the original ids kept exactly (`fx-431` stays
`fx-431`):

| bd field | in seeds |
|---|---|
| `id`, `title`, `description`, `status`, `priority`, `issue_type` | the seed's modelled fields |
| `assignee`, `owner`, `created_by` | modelled fields |
| `labels` | labels |
| `dependencies` (`blocks`, `parent-child`, `related`, `discovered-from`) | edges `ready`, `blocked` and `graph` read |
| `comments` | comments, in order, with author and time |
| `close_reason`, `closed_at`, `created_at`, `updated_at` | modelled fields |
| anything else (`started_at`, `metadata`, the `*_count` fields, `_type`, unknown edge types) | carried verbatim; `sd cutover export` gives it back |

Re-running the import is idempotent: a second `cutover import` of the same
file reports `"wrote": false` with no created or updated ids. `cutover
verify` reports `"losses": 0` for the bd 1.0.5 export, and `cutover export`
returns the same records value for value (keys in sorted order). The test
`bd_dolt_export_imports_losslessly_and_idempotently` in `tests/cli.rs` pins
all of this against a fixture exported by bd 1.0.5
(`tests/fixtures/bd-1.0.5-export.jsonl`).

`bd export --all` also writes records that are not issues (memories, tagged
`_type: "memory"`). seeds refuses such a file with exit 2 and writes nothing,
rather than dropping them; export without `--all`.

Not imported: Dolt branches and commit history, bd's event log, and the
non-issue tables `bd export` omits. History in seeds starts at the import
transaction.

## bd to sd commands

Audited against `bd help` for bd 1.0.5 and `sd <verb> --help`, each row run
against a scratch store. Where seeds follows `br` and bd differs, br wins
(see the [intent map](intent-map.md)).

| bd | sd | Status |
|---|---|---|
| `bd create "T" -p 1 -t task -d D -l a,b -a NAME --parent ID --deps ID` | same | works |
| `bd create … --acceptance TEXT` | same (alias of `--acceptance-criteria`) | works |
| `bd show ID [ID…] --json` | same | works; `--as-of` is `--at <tx>` |
| `bd list -s open -t task -p 1 -l L --json` | same | works |
| `bd list -a NAME` | `sd list --assignee NAME` | differs: `-a` is `--all` (br) |
| `bd list -n 10`, `bd ready -n 10` | `--limit 10` | differs: no `-n` short form |
| `bd ready --json` | same | works |
| `bd ready -u` | `sd ready --unassigned` | differs: no `-u` short form |
| `bd update ID --claim` / `--status S` / `--add-label L` / `--remove-label L` | same | works |
| `bd update ID --notes N --design D --acceptance A` | same | works |
| `bd close ID -r "why"` | same | works; refuses while open blockers remain unless `--force` |
| `bd close ID --reason-file F` | `-r "$(cat F)"` | differs: no `--reason-file` |
| `bd reopen ID -r "why"` | same | works |
| `bd dep add A B` (A depends on B) | same | works; `-t related` and the other types too |
| `bd dep add A --blocked-by B` | `sd dep add A B` | differs: positional only |
| `bd dep remove A B` | same | works |
| `bd dep tree ID` | `sd graph ID` (`--dependencies` for what it waits on) | differs |
| `bd comments add ID "text"` | same (`-m`, `-f` too) | works |
| `bd comments ID` | `sd comments list ID` | differs: listing needs `list` |
| `bd create --notes N`, `bd create --design D` | `sd update ID --notes N --design D` after create | differs |
| `bd blocked`, `bd stats`, `bd search Q` | same | works |
| `--json` on every verb above | same, bd's key set | works (pinned in `tests/json_schema.rs`) |

`sd capabilities --json` lists what the installed build supports.

## Not in seeds

bd's Dolt plumbing (`bd dolt …`, `bd backup …`, `bd vc …`) has no seeds
equivalent: a seeds ledger is a quipu graph, shared as a
[pendant](qpack-sharing.md) or a server, not a Dolt database. Workflow
molecules and formulas are shuttle's ([Formulas](formulas.md)).
