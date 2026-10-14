# Configured due dates

A project may choose creation deadlines in `.seeds/config.toml`:

```toml
[due_dates]
priority_days = [1, 3, 14, 30, 90] # calendar days, P0 through P4
```

The project setting overrides the user setting. Each value must be positive
and at most 36500. An absent table leaves existing projects unchanged.
`create`, `q` and bulk markdown creation use the configured deadline when no
explicit due value is supplied. An empty due input uses the configured default.
Explicit dates win. Inbox pointers (`inbox:` or
`mail:` titles, or the `inbox` label) do not receive defaults.

Changing or clearing an existing date requires a nonempty reason committed
with the change:

```sh
sd update task-id --due +3d --transition-comment 'Revised delivery estimate'
```

Setting the same date is idempotent. This guard also applies to multi-item
updates; a refused item prevents the entire batch.

## Existing undated work

Generate a read-only packet, review its membership and computed dates, then
apply exactly that packet:

```sh
sd due-backfill --plan-output reviewed-due-plan.json --json
sd due-backfill --apply-plan reviewed-due-plan.json --json
```

The plan includes only open, in-progress and blocked items with no due date,
excluding inbox pointers. Dates are derived from creation time and configured
priority days. Existing dates are preserved. An unreadable creation date
refuses planning. The packet binds graph, configuration, item revisions and
computed dates; changed rows, duplicate IDs, foreign graphs and edited dates
refuse before writing. Application is one atomic revision-checked batch, not a
write loop. Transport request limits remain in force; a refused oversized plan
must be retained and reviewed, not silently split or retried. The output path
must be new so an earlier reviewed packet cannot be overwritten.
