# Agent orientation with sd prime

`sd prime` prints the embedded agent primer, followed by a bounded read-only
view of the configured board. `--static` skips configuration, files and network;
`--brief` prints five to ten lines; `--section <prefix>` selects a heading.
Unknown sections or flags exit 2. Missing, unreachable, unreadable and slow
boards retain the primer and exit 0 with a degraded summary.

`--json` uses `stack-prime/1`: `tool`, `version`, `primer.sections`,
`primer.text`, `live.{status,elapsed_ms,summary}` and overall `status`.
`--brief --json` omits `primer.text`; `--static` reports live `skipped` and an
empty summary. Overall status is `ok` for live `ok` or `skipped`, `degraded`
otherwise. Unknown counts are null rather than zero. This orientation envelope
is separate from the item JSON used by `sd list` and `sd show`.

The plate uses `--actor` / `SEEDS_ACTOR`, then `SHANTY_AGENT`, then the usual
user fallback. It lists at most ten assigned active items, including deferred
ones, with a total count and title previews capped at 160 characters. The ready
count is scoped to that same actor before backend reads, using the same dependency,
deferral and ephemeral-item rules as `sd ready --assignee <agent>`. It does not
count unassigned work or another agent's queue. A one-row response limit is not
a server-side work bound; actor scoping avoids a board-wide readiness query. This is a read-time view, not a claim
of continuously fresh ownership. Plate and ready are separate reads, not a
transactional combined snapshot. Replica notices remain in the summary.
A remote historical `--at` summary is unavailable (null counters, degraded)
because that engine path currently requires a whole-board snapshot; explicit
`sd list/ready --at` remain available. Local historical reads are unchanged.

The live probe runs inside a 750 ms parent deadline. Slow configuration,
connection, authentication or queries become `partial`; the parent kills and
reaps its read-only worker. Responses are capped at 64 KiB. The probe never
initializes a project, generates a project ID, imports a pendant, syncs,
enrolls a key or opens a writable backend. Local reads summarize the existing
working store; an unreconciled pendant may differ. Configure a graph or project
identity deliberately before expecting board counts.

Use `sd where --json` to inspect routing and `sd doctor` to diagnose failures.
Credentials retain normal destination policy and are never placed in worker
arguments or echoed from error messages. No fallback to a different board occurs.

The primer source is `src/native/primer.md`; the skill copy is generated from
it and the normal CI suite checks both copies. Regenerate the skill with
`SEEDS_BLESS_PRIMER=1 just prime test`.

{{#include ../../../src/native/primer.md}}
