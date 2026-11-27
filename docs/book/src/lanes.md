# Lanes and WIP caps

A seed's `lane` says where it sits in the flow. Priority stays severity; rank
orders work inside a priority; the lane is the column.

| lane | what it holds | default cap |
|---|---|---|
| `triage` | unrefined intake, emptied by refinement | uncapped |
| `later` | refined, epic-assigned work | 150 |
| `next` | ready work, about two weeks of throughput | 40 |
| `now` | in the sprint | 10 in total, and 2 per assignee |
| `icebox` | parked, with a reason and a review date or event | never counted |

```bash
sd update sd-abc --lane triage
sd update sd-abc --lane now --assignee wu
sd update sd-abc --lane icebox --icebox-reason "waits on quipu v3" --icebox-review "when v3 ships"
sd list --lane next
```

## Caps are enforced on write

Every write that puts an item INTO a capped lane is checked: a new lane, a
closed item reopened inside one, or (for Now's per-assignee cap) an item
reassigned to someone else inside Now. Editing or reranking an item already in
its lane is never refused. When the lane is full the write is refused (exit
code of a refusal, nothing written) and the message names what could leave and
the only ways out, the dispositions:

- done (`sd close <id> --reason <evidence>`)
- duplicate-of `<id>`
- superseded-by `<id>`
- won't-do (`sd close <id> --reason <why>`)
- iceboxed (`--lane icebox` with a reason and a review)

`--json` carries the same refusal as `details` with schema
`seed-lane-cap/v1`: `lane`, `scope`, `cap`, `entering`, `could_leave` and
`dispositions`.

## The icebox

The icebox parks work without deciding it away. Moving an item there needs
both `--icebox-reason` and `--icebox-review` (a date or an event, as text);
without them the write is a usage error. Iceboxed items hold no lane slot, so
parking a Now item frees its slot. Moving an item out of the icebox clears its
reason and review.

## Caps are data, from two sources

The defaults above apply until a cap is set. A cap can be set in either of two
places:

- **the project file**: a `[lanes]` table in `.seeds/config.toml` (the user
  file may set one too; the project file wins per key):

  ```toml
  [lanes]
  later = 150
  now = 10
  now_per_assignee = 2
  next = 40
  triage = 0      # 0 = uncapped
  ```

- **the project graph**: `seeds:laneCap` literals on the board node, the
  project graph's IRI followed by `#board`, each `key=value`
  (`"later=150"`). These travel with the graph, so every checkout and every
  server reads the same caps.

A key set in only one source takes that value. A key set in both must agree:
two different values are a conflict, reported with schema
`seed-lane-cap-conflict/v1` and nothing written, never resolved by picking a
side. The keys are `now`, `now_per_assignee`, `next`, `later` and `triage`;
any other key is refused.

The core reads the caps through the `Backend` (`lane_cap_config`,
`lane_cap_graph`) and resolves them with `model::LaneCaps::resolve`, so the
rule is the same on the embedded store, a server and wasm.
