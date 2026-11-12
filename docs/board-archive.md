# The board archive

A remote board keeps closed seeds forever, so every board read (`ready`, `list`, `stats`) used to scan closed history as well as live work. The board archive moves closed seeds past an age cutoff, with their comments, out of the live project graph into `<graph>#seeds-archive` in the same store. Their IRIs do not change.

## What reads see

| Read | Live graph | Archive |
|---|---|---|
| `ready`, `list`, `blocked`, `count` | yes | no |
| `stats` | counts and lead time | `archived_issues` only, counted in nothing else |
| `show <id>`, current state | first | when the live graph lacks the id; listed in `archived` |
| an edge to an archived seed in `show` | | status read from the archive (`closed`), never `null` |
| `show --at <tx>` | history only | no |

An archived blocker never blocks: readiness only counts a blocker whose status is not `closed` or `tombstone`, and an archived seed has no status in the live graph.

Known limits:

- `show` reads dependency **targets** from the archive, not **dependents**. A closed, archived seed that depended on the shown seed is not listed in its `dependents`. It is closed, so it changes nothing about readiness.
- The live read and the archive read are separate observations. A seed archived between them is briefly `NOT_FOUND` to that one `show`. Archived seeds are immutable, so a second read is correct.

## Writes

An archived seed is read-only. A write that names one (`update`, `close`, `comments add`, `dep add` onto it, `create --parent` or `--deps` naming it) is refused with `sd unarchive <id> --execute` as the way back, and nothing is written. Nothing is ever recreated in the live graph as a partial seed.

An archived seed keeps its id: `create` never mints an id the archive holds, and a keyed (run + step) create finds the archived seed as the existing one.

## Moving seeds

```text
sd archive --closed-before 2026-10-28T00:00:00Z --protect protect.json            # a plan; moves nothing
sd archive --closed-before 2026-10-28T00:00:00Z --protect protect.json \
           --batch 100 --execute --log archive.jsonl                               # move
sd unarchive --from-log archive.jsonl --execute                                    # undo exactly those batches
```

Selection keeps a seed live when any of these hold:

- it is named in the protect file (`items[].id`), or carries a label in `protected_labels` or starting with one of `protected_label_prefixes`;
- a seed that stays live (not closed, or closed on or after the cutoff) has a dependency edge to it;
- it is named by `closed:<id>` in a live seed's `resume_when:` line.

Each batch is one update. Its branches name each seed as a bound IRI, never `VALUES` (not pushed down, so a whole-graph scan) and never a `BIND` of an outer variable (unbound inside the nested group, so it matches the whole graph). A forward branch also requires the seed to still be `closed`, so a seed reopened since selection stays. After each update the batch is verified: the facts read from the source before the move must be exactly the facts in the destination after it, with none left in the source. A mismatch stops the run before the next batch. A lost response is never resent: the verification read decides whether it landed.

A writer with exact graph scope needs the archive graph in its grant.
