# Replace a specification, then acknowledge it

`rescope` replaces the description with one consolidated `SPEC vN`. It appends
a timestamped comment containing the verbatim request, requester, acting
principal, summary and declared session. Description, generation and comment
land in one revision-checked transaction. Prior descriptions remain readable
with `sd show ID --at TX`; `sd history ID` identifies those transactions.

```sh
sd rescope work-123 --spec-file new.md --quote "Please limit this to the retention audit." \
  --requester operator --summary "retention audit" --session session-123
sd spec-state work-123 --json
sd ack work-123 --version 1 --session session-123
```

The assignee acknowledges the exact generation it read. Another assignee,
a stale generation or a conflicting receipt refuses. Repeating an existing ACK
returns a null transaction. An ordinary description update cannot bypass
rescope once a description has a governed generation.

The description is limited to 4,096 UTF-8 bytes, including its generated heading.
Configure `[specs] byte_cap` to change this bound. Empty specifications,
oversized input and layering markers (`ADDITION`, `SCOPE EXPANSION`,
`supersedes the above`) refuse before writing. A replacement file may carry a
version heading; the writer computes the authoritative generation. Another
requester's change requires a non-empty quote. `--dry-run` validates the same
request and returns the planned generation without committing it. A declared
session describes planned provenance in this mode; other read commands retain
their attribution guard.

## Notifications and awareness are separate

`spec-events --json` returns retained pending notification intents. Their stable
identity is item plus spec generation; polling and ordinary edits cannot mint a
new generation. `spec-receipt ID VERSION --json` reads a retained delivery
receipt even after the intent stops being pending. The shared notifier confirms
its verified inbox receipt with `spec-receipt-ack ID VERSION --receipt RECEIPT`.
A response lost after this write is reconciled by exact receipt readback. A
notification receipt does not acknowledge the specification.

`spec-changes --json` lists unacknowledged changes, including delivered pointers.
These delivery reads are current-state operations and refuse `--at`.
`spec-state` and `show` support historical pins.

An owner start/claim transition carrying a declared session records the spec
generation at that transition. A lead changing somebody else's status does not
invent an owner start. `seed_spec_awareness` is the portable stored query in
`queries/spec-awareness.json`; `sd spec-query-definition` emits its definition
for Quipu's validated registry. Load it through the reviewed registry or share
projection, then call it with the exact graph and item IRIs. It returns the
recorded start generation, current generation and explicit owner ACK. Missing
start evidence is `UNKNOWN`. Its private-store control proves start v1, current
v2 and ACK v2 independently. Query availability on a running server requires
registry projection; source publication alone does not install it.

When using Shantytown, `st work item rescope` and `st work item ack` provide the
same operations on the selected Seeds backend. The existing capped briefing
shows unacknowledged generations and the administrator sees their age. The
existing scheduled notifier owns pointer delivery. Default pointers wait for
the recipient's next Stop boundary; urgent P0 pointers use the normal live
input lane. The recipient adapter reads bounded spec metadata and fences each
generation before emitting, so an uncertain hook output cannot repeatedly wake
an owner. UNKNOWN reads remain UNKNOWN, and the session ceiling outranks this
notice. Enable the reviewed `[overdue] spec_changes` configuration only after
the corresponding SDK and hook are installed; it defaults to false.

Pause governed spec edits during rollback, preserve items, history and receipts,
and remove new configuration before downgrading a parser that does not know it.
An older client does not enforce the new replacement protocol. Scheduled
receipt, actual recipient arrival and installed identity are separate acceptance
proofs.
