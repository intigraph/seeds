---
name: seeds
description: Orient on a Seeds work board with sd prime, inspect the configured scope, and use Seeds lifecycle verbs.
---

Run `sd prime` for the embedded primer and a bounded read-only board summary.
Use `--static` for offline orientation and `--json` for structured output.
Verify the configured board with `sd where --json` before changing work.
Read exact items and preserve conflicts and policy refusals.

## Agent primer

<!-- BEGIN sd prime -->
### Seeds agent primer

#### What Seeds is

Seeds is the work-item tracker for the Quipu stack; its command is `sd`.
Items and their fields are versioned facts in a Quipu knowledge graph.
A named graph identifies each board; storage can be local or remote.
Camayoc owns shared vocabulary and definitions; Shuttle owns workflows.
Desire Path helps callers move from compatible `bd` and `br` verbs to Seeds.

#### Agent workflow

1. Run `sd where --json` to identify the configured board; keep its graph scope.
2. Read your plate with `sd list --assignee <agent>` and inspect an exact item with `sd show <id>`.
3. Use `sd ready` for work whose dependencies and deferrals permit it; an error is not an empty board.
4. Claim with `sd update <id> --claim`; a conflict means somebody else changed it.
5. Record evidence, close with a reason, and read back the resulting item.

#### Core commands

| Command | Use |
| --- | --- |
| `sd prime` | This primer, your plate and a bounded board summary |
| `sd where --json` | Resolved store or server and named graph |
| `sd list --assignee <agent>` | Assigned active work |
| `sd ready` | Work permitted by dependencies and deferrals |
| `sd show <id>` | Exact item and current facts |
| `sd create "<title>"` | Create a work item |
| `sd update <id> --claim` | Compare-and-set claim |
| `sd close <id> --reason "<evidence>"` | Close with a reason |
| `sd dep add <id> <blocker>` | Declare a dependency |
| `sd comments add <id> "<evidence>"` | Attach evidence |
| `sd capabilities --json` | Supported graph vocabulary and operations |
| `sd doctor` | Diagnose a configured board |

#### MCP tools

None: Seeds exposes the `sd` CLI and a WASM-clean library. Quipu provides
knowledge-graph MCP tools; use Seeds verbs for work-item lifecycle rules.

#### Next steps

- Book: <https://intigraph.github.io/seeds/>
- CLI reference: `docs/book/src/reference.md` and `sd help <command>`
- Agent skill: `skills/seeds/SKILL.md`
- Storage and configuration: `docs/book/src/storage-modes.md` and `docs/book/src/config.md`
<!-- END sd prime -->
