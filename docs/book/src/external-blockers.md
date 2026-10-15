# External prerequisites

A seed can wait for a node in another registered Quipu graph. This is separate
from an ordinary seed-to-seed dependency; the foreign node is never retyped as
a local seed.

```sh
sd dep add example-1 --blocker urn:example:release --done-when published
sd dep add example-2 --blocker 'Approval decision' --blocker-graph urn:example:decisions --done-when decided
sd dep add example-3 --blocker urn:example:service --blocker-graph urn:example:operations --ask 'ASK { <urn:example:service> <urn:example:healthy> true }'
```

Exact labels and graph discovery must be unambiguous. If a node occurs in
several graphs, declare `--blocker-graph`. The stored graph is pinned; later
reconciliation does not silently switch datasets. Without `--done-when` or
`--ask`, supported exact types select closed (Action/WorkItem), merged (PR),
published (Release), exists (Directive), or decided (Decision). An unknown or
ambiguous type requires an explicit condition. Closed without merged is not a
completed PR. IRI strings are validated before generating queries.

```sh
sd blockers reconcile --json
sd show example-1 --json
sd graph example-1 --dependencies --json
sd blockers events --json
```

Reconciliation holds an open seed while any condition is false or unreadable.
Only a feature-owned blocked status is automatically restored to open; manual
status edits, date holds and other prerequisites retain authority. Ready and
claim also reread external conditions, so a stale done observation cannot
authorize work. Show and graph distinguish recorded state from current state.

There is no notification sender in Seeds. A shared consumer reads persistent
pending unlock intents, retains its delivery receipt, and acknowledges it:

```sh
sd blockers ack example-1 seeds:unlock:example-1:1 --receipt retained-receipt-1 --json
sd blockers receipt example-1 seeds:unlock:example-1:1 --json
```

Generation identifies the actual readiness episode and survives ordinary edits.
An unknown delivery must remain pending. If ACK is uncertain, read the retained
receipt before a byte-identical retry; queue absence alone is not evidence.
No schedule is armed by declaring an edge: configure the shared consumer and
reconciler through your installation's normal schedule lane.

Remove an external prerequisite by exact IRI, then reconcile:

```sh
sd dep remove example-1 --blocker urn:example:release
sd blockers reconcile --json
```

Legacy migration first reports its count and exact revision-bound plan:

```sh
sd blockers migrate --dry-run --json
sd blockers migrate --execute --json
```

Only active `resume_when: closed:<id>` lines on deferred seeds qualify. Prose
mentions are not conditions. Multiple conditions, independent date holds,
unreadable targets and duplicate declarations refuse execution of the entire
batch. Execution is one validated revision-checked transaction, not a sequence
of per-item writes. It preserves other notes and records normal write provenance.
Review the plan before executing through your governed migration lane.
